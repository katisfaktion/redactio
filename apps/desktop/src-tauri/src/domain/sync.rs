use super::{
    audit::{append_audit, AuditAction, AuditEntry},
    mapping::{
        commit_generation, recover_pending, CollectionGuard, CommitCandidate, DocumentState,
        Mapping, ReviewRecord,
    },
    scan::{hash_output, scan_collection, ScanFailureOrigin, ScanReport, ScannedFile},
    settings::{load_settings, ProcessingFingerprint, SyncPair},
    storage::ValidatedWrite,
};
use crate::{
    error::AppError,
    protocol::{
        ConfigurePayload, ConfigureResult, Decisions, DocumentKey, EngineInfo, ProcessRequest,
        ProcessResult, ReviewRequest, ReviewStatus,
    },
    sidecar::{Sidecar, DOCUMENT_TIMEOUT, INITIALIZATION_TIMEOUT},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCounts {
    pub discovered: u64,
    pub processed: u64,
    pub skipped: u64,
    pub failed: u64,
    pub unprocessed: u64,
    pub warned: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunOutcome {
    Completed,
    CompletedWithErrors,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStage {
    Initializing,
    Scanning,
    Processing,
    Finished,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunProgress {
    pub sync_pair_id: Uuid,
    pub run_id: Uuid,
    pub stage: RunStage,
    #[serde(flatten)]
    pub counts: RunCounts,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunFailure {
    pub relative_path: String,
    #[serde(flatten)]
    pub error: AppError,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    #[serde(flatten)]
    pub progress: RunProgress,
    pub outcome: RunOutcome,
    pub errors: Vec<RunFailure>,
    pub error: Option<AppError>,
    pub audit_warning: bool,
}

#[derive(Clone)]
pub struct RunController {
    inner: Arc<RunState>,
}
struct RunState {
    settings_path: PathBuf,
    operation: Arc<AsyncMutex<()>>,
    active: Mutex<Option<ActiveRun>>,
    summaries: Mutex<HashMap<(Uuid, Uuid), RunSummary>>,
}
struct ActiveRun {
    pair_id: Uuid,
    run_id: Uuid,
    cancelled: Arc<AtomicBool>,
}

/// Lock order is app operation, app config, then source. All survive awaits and
/// all exits release them, including a dropped worker future.
pub struct OperationGuard {
    source: CollectionGuard,
    config: CollectionGuard,
    _operation: OwnedMutexGuard<()>,
}
pub struct BatchRun {
    pub run_id: Uuid,
    pair: SyncPair,
    paths: Option<Vec<String>>,
    force: HashSet<String>,
    cancelled: Arc<AtomicBool>,
    controller: RunController,
    guard: OperationGuard,
    started_at: String,
    engine: Option<ProcessingFingerprint>,
}

impl Drop for BatchRun {
    fn drop(&mut self) {
        if let Ok(mut active) = self.controller.inner.active.lock() {
            if active.as_ref().is_some_and(|run| run.run_id == self.run_id) {
                *active = None;
            }
        }
    }
}

impl RunController {
    pub fn new(settings_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(RunState {
                settings_path,
                operation: Arc::new(AsyncMutex::new(())),
                active: Mutex::new(None),
                summaries: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Reused by pair mutations, configuration, review saves and export.
    pub fn try_operation(&self) -> Result<OwnedMutexGuard<()>, AppError> {
        self.inner
            .operation
            .clone()
            .try_lock_owned()
            .map_err(|_| AppError {
                code: "operation_busy".into(),
                retryable: true,
            })
    }

    pub fn scan(&self, pair_id: Uuid) -> Result<ScanReport, AppError> {
        let (pair, _guard) = self.lock_pair(pair_id)?;
        #[cfg(test)]
        tests::pause_scan();
        scan_collection(&pair)
    }

    fn lock_pair(&self, pair_id: Uuid) -> Result<(SyncPair, OperationGuard), AppError> {
        let operation = self.try_operation()?;
        let root = self
            .inner
            .settings_path
            .parent()
            .ok_or_else(|| AppError::new("invalid_path"))?;
        let config = CollectionGuard::acquire(root)?;
        let settings = load_settings(&self.inner.settings_path)?;
        let pair = settings
            .sync_pairs
            .iter()
            .find(|pair| pair.id == pair_id)
            .cloned()
            .ok_or_else(|| AppError::new("unknown_pair"))?;
        settings.validate_roots_with(&[root.to_path_buf()])?;
        let source = CollectionGuard::acquire(&pair.source_folder)?;
        settings.validate_registry(&[root.to_path_buf()])?;
        Ok((
            pair,
            OperationGuard {
                _operation: operation,
                config,
                source,
            },
        ))
    }

    pub fn prepare(
        &self,
        pair_id: Uuid,
        paths: Option<Vec<String>>,
        force: Vec<String>,
    ) -> Result<BatchRun, AppError> {
        let (pair, guard) = self.lock_pair(pair_id)?;
        let run_id = Uuid::new_v4();
        let cancelled = Arc::new(AtomicBool::new(false));
        *self
            .inner
            .active
            .lock()
            .map_err(|_| AppError::new("state_unavailable"))? = Some(ActiveRun {
            pair_id,
            run_id,
            cancelled: cancelled.clone(),
        });
        Ok(BatchRun {
            run_id,
            pair,
            paths,
            force: force.into_iter().collect(),
            cancelled,
            controller: self.clone(),
            guard,
            started_at: now(),
            engine: None,
        })
    }

    pub fn request_cancel(&self, pair_id: Uuid, run_id: Uuid) -> Result<(), AppError> {
        let active = self
            .inner
            .active
            .lock()
            .map_err(|_| AppError::new("state_unavailable"))?;
        let run = active
            .as_ref()
            .filter(|run| run.pair_id == pair_id && run.run_id == run_id)
            .ok_or_else(|| AppError::new("unknown_run"))?;
        run.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub async fn cancel(&self, pair_id: Uuid, run_id: Uuid) -> Result<(), AppError> {
        // Finish the current bounded request; its existing sidecar deadline
        // kills a stall. Cancellation prevents scheduling the next document.
        self.request_cancel(pair_id, run_id)
    }

    pub fn summary(&self, pair_id: Uuid, run_id: Uuid) -> Result<Option<RunSummary>, AppError> {
        if !load_settings(&self.inner.settings_path)?
            .sync_pairs
            .iter()
            .any(|pair| pair.id == pair_id)
        {
            return Err(AppError::new("unknown_pair"));
        }
        Ok(self
            .inner
            .summaries
            .lock()
            .map_err(|_| AppError::new("state_unavailable"))?
            .get(&(pair_id, run_id))
            .cloned())
    }

    pub async fn execute(
        &self,
        mut run: BatchRun,
        sidecar: Result<Sidecar, AppError>,
        mut emit: impl FnMut(&RunProgress),
    ) -> RunSummary {
        let mut summary = RunSummary {
            progress: RunProgress {
                sync_pair_id: run.pair.id,
                run_id: run.run_id,
                stage: RunStage::Initializing,
                counts: RunCounts::default(),
            },
            outcome: RunOutcome::Failed,
            errors: vec![],
            error: None,
            audit_warning: false,
        };
        let result = self
            .process(&mut run, sidecar, &mut summary, &mut emit)
            .await;
        summary.outcome = if run.cancelled.load(Ordering::SeqCst) {
            RunOutcome::Cancelled
        } else if result.is_err() {
            RunOutcome::Failed
        } else if summary.progress.counts.failed > 0 {
            RunOutcome::CompletedWithErrors
        } else {
            RunOutcome::Completed
        };
        summary.error = result
            .err()
            .filter(|error| error.code != "operation_cancelled");
        summary.progress.stage = RunStage::Finished;
        // Every increment consumes exactly one unprocessed document. Fail closed
        // before serializing if a future orchestration change breaks conservation.
        if check_counts(&summary.progress.counts).is_err() {
            summary.outcome = RunOutcome::Failed;
            summary.error = Some(AppError::new("invalid_run_counts"));
            summary.progress.counts = RunCounts {
                discovered: summary.progress.counts.discovered,
                unprocessed: summary.progress.counts.discovered,
                ..RunCounts::default()
            };
        }
        let mut codes: Vec<_> = summary
            .errors
            .iter()
            .map(|failure| failure.error.code.clone())
            .collect();
        if let Some(error) = &summary.error {
            codes.push(error.code.clone());
        }
        codes.sort();
        codes.dedup();
        let entry = AuditEntry {
            schema_version: 1,
            sync_pair_id: run.pair.id,
            run_id: run.run_id,
            action: AuditAction::Sync,
            started_at: run.started_at.clone(),
            finished_at: now(),
            outcome: summary.outcome,
            counts: summary.progress.counts,
            processing_revision: run.pair.processing_revision,
            engine: run.engine.clone(),
            error_codes: codes,
        };
        summary.audit_warning =
            append_audit(self.inner.settings_path.parent().unwrap(), &entry).is_err();
        self.inner
            .summaries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert((run.pair.id, run.run_id), summary.clone());
        drop(run);
        emit(&summary.progress);
        summary
    }

    async fn process(
        &self,
        run: &mut BatchRun,
        sidecar: Result<Sidecar, AppError>,
        summary: &mut RunSummary,
        emit: &mut impl FnMut(&RunProgress),
    ) -> Result<(), AppError> {
        publish(&summary.progress, emit)?;
        cancelled(run)?;
        let sidecar = sidecar?;
        let configured = configure_pair(&run.pair, &sidecar).await?;
        run.engine = Some(fingerprint(&configured.engine));
        cancelled(run)?;
        summary.progress.stage = RunStage::Scanning;
        publish(&summary.progress, emit)?;
        let mut mapping = Mapping::load(
            &run.pair.source_folder,
            run.pair.id,
            &run.pair.target_folder,
        )?;
        match recover_pending(
            &run.pair,
            &mut mapping,
            &run.guard.source,
            &self.inner.settings_path,
            &run.guard.config,
        ) {
            Ok(()) => (),
            Err(error) if error.code == "output_conflict" => (),
            Err(error) => return Err(error),
        }
        let mut report = scan_collection(&run.pair)?;
        let known: HashSet<_> = report
            .files
            .iter()
            .map(|file| file.relative_path.as_str())
            .chain(
                report
                    .errors
                    .iter()
                    .map(|error| error.relative_path.as_str()),
            )
            .collect();
        if run
            .paths
            .as_ref()
            .is_some_and(|paths| paths.iter().any(|path| !known.contains(path.as_str())))
            || run.force.iter().any(|id| {
                !report.files.iter().any(|file| {
                    file.doc_id.as_ref() == Some(id)
                        && run
                            .paths
                            .as_ref()
                            .is_none_or(|paths| paths.contains(&file.relative_path))
                })
            })
        {
            return Err(AppError::new("invalid_selection"));
        }
        if let Some(paths) = &run.paths {
            report
                .files
                .retain(|file| paths.contains(&file.relative_path));
            report
                .errors
                .retain(|error| paths.contains(&error.relative_path));
        }
        let mut scan_errors: HashMap<_, _> = report
            .errors
            .into_iter()
            .map(|error| (error.relative_path.clone(), error))
            .collect();
        let extra_errors = scan_errors
            .keys()
            .filter(|path| !report.files.iter().any(|file| &file.relative_path == *path))
            .count();
        let discovered = report
            .files
            .len()
            .checked_add(extra_errors)
            .and_then(|count| u64::try_from(count).ok())
            .ok_or_else(|| AppError::new("invalid_run_counts"))?;
        summary.progress.counts = RunCounts {
            discovered,
            unprocessed: discovered,
            ..RunCounts::default()
        };
        summary.progress.stage = RunStage::Processing;
        publish(&summary.progress, emit)?;
        for file in &report.files {
            cancelled(run)?;
            let scan_error = scan_errors.remove(&file.relative_path);
            let scan_origin = scan_error.as_ref().map(|error| error.origin);
            let result = if let Some(failure) = scan_error {
                Err(AppError {
                    code: failure.code,
                    retryable: true,
                })
            } else {
                process_file(run, &mut mapping, file, &sidecar, &configured.engine).await
            };
            if run.cancelled.load(Ordering::SeqCst) && result.is_err() {
                return Err(AppError::new("operation_cancelled"));
            }
            summary.progress.counts.unprocessed -= 1;
            match result {
                Ok(None) => summary.progress.counts.skipped += 1,
                Ok(Some(warned)) => {
                    summary.progress.counts.processed += 1;
                    summary.progress.counts.warned += u64::from(warned);
                }
                Err(error) => {
                    summary.progress.counts.failed += 1;
                    summary.errors.push(RunFailure {
                        relative_path: file.relative_path.clone(),
                        error: error.clone(),
                    });
                    publish(&summary.progress, emit)?;
                    if scan_origin == Some(ScanFailureOrigin::Output)
                        || (scan_origin.is_none() && !individual_error(&error.code))
                    {
                        return Err(error);
                    }
                    continue;
                }
            }
            publish(&summary.progress, emit)?;
        }
        for (relative_path, failure) in scan_errors {
            cancelled(run)?;
            summary.progress.counts.unprocessed -= 1;
            summary.progress.counts.failed += 1;
            let error = AppError {
                code: failure.code,
                retryable: true,
            };
            summary.errors.push(RunFailure {
                relative_path,
                error: error.clone(),
            });
            if failure.origin == ScanFailureOrigin::Output {
                publish(&summary.progress, emit)?;
                return Err(error);
            }
        }
        publish(&summary.progress, emit)?;
        Ok(())
    }
}

fn configuration(pair: &SyncPair) -> ConfigurePayload {
    ConfigurePayload {
        sync_pair_id: pair.id,
        processing_revision: pair.processing_revision,
        config: pair.config.clone(),
    }
}
fn now() -> String {
    request_timestamp(OffsetDateTime::now_utc())
}

fn request_timestamp(instant: OffsetDateTime) -> String {
    // Python datetime carries microseconds; do not send precision it cannot echo.
    instant
        .replace_nanosecond(instant.microsecond() * 1000)
        .expect("valid microseconds")
        .format(&Rfc3339)
        .expect("UTC timestamp is representable")
}
fn cancelled(run: &BatchRun) -> Result<(), AppError> {
    if run.cancelled.load(Ordering::SeqCst) {
        Err(AppError::new("operation_cancelled"))
    } else {
        Ok(())
    }
}
fn check_counts(counts: &RunCounts) -> Result<(), AppError> {
    if !counts.is_consistent() || counts.discovered > 9_007_199_254_740_991 {
        Err(AppError::new("invalid_run_counts"))
    } else {
        Ok(())
    }
}
fn publish(progress: &RunProgress, emit: &mut impl FnMut(&RunProgress)) -> Result<(), AppError> {
    check_counts(&progress.counts)?;
    emit(progress);
    Ok(())
}

fn individual_error(code: &str) -> bool {
    matches!(
        code,
        "invalid_docx"
            | "unsafe_xml"
            | "unreadable_document"
            | "document_too_large"
            | "archive_too_large"
            | "archive_expansion_limit"
            | "text_too_large"
            | "unsupported_document"
            | "encrypted_document"
            | "source_changed"
            | "output_conflict"
            | "review_conflict"
            | "confirmation_required"
            | "recovery_pending"
            | "missing_source"
    )
}

async fn process_file(
    run: &BatchRun,
    mapping: &mut Mapping,
    file: &ScannedFile,
    sidecar: &Sidecar,
    engine: &EngineInfo,
) -> Result<Option<bool>, AppError> {
    if file.state == DocumentState::MissingSource {
        return Err(AppError::new("missing_source"));
    }
    if file.state == DocumentState::Conflict {
        return Err(AppError::new("output_conflict"));
    }
    let existing = mapping
        .entries()
        .iter()
        .find(|entry| entry.relative_path == file.relative_path)
        .cloned();
    let source_hash = file
        .source_hash_sha256
        .as_ref()
        .ok_or_else(|| AppError::new("missing_source"))?;
    let source_path = run.pair.source_folder.join(&file.relative_path);
    let source_witness = ValidatedWrite::new(&run.pair.source_folder, &source_path)
        .map_err(|_| AppError::new("unreadable_document"))?;
    if hash_output(&run.pair.source_folder, &source_path)
        .map_err(|_| AppError::new("unreadable_document"))?
        .as_ref()
        != Some(source_hash)
    {
        return Err(AppError::new("source_changed"));
    }
    if let Some(entry) = &existing {
        if let Some(generation) = &entry.committed {
            if entry.pending.is_none() {
                let key = DocumentKey {
                    sync_pair_id: run.pair.id,
                    doc_id: entry.doc_id.clone(),
                };
                let review = ReviewRecord::read(&run.pair.source_folder, &key, generation)
                    .map_err(|_| AppError::new("review_conflict"))?;
                if file.state == DocumentState::Current && !run.force.contains(&entry.doc_id) {
                    let output = run.pair.target_folder.join(format!("{}.md", entry.doc_id));
                    if hash_output(&run.pair.target_folder, &output)
                        .map_err(|_| AppError::new("output_conflict"))?
                        .as_ref()
                        != Some(&generation.output_hash)
                    {
                        return Err(AppError::new("output_conflict"));
                    }
                    source_witness
                        .validate()
                        .map_err(|_| AppError::new("source_changed"))?;
                    return Ok(None);
                }
                if (review.reviewed_at.is_some()
                    || !review.decisions.manual.is_empty()
                    || !review.decisions.dismissed_ids.is_empty()
                    || !review.notes.is_empty())
                    && !run.force.contains(&entry.doc_id)
                {
                    return Err(AppError::new("confirmation_required"));
                }
            }
        }
    }
    let doc_id = mapping.reserve_with_guard(&file.relative_path, &run.guard.source)?;
    let key = DocumentKey {
        sync_pair_id: run.pair.id,
        doc_id: doc_id.clone(),
    };
    let output_path = run.pair.target_folder.join(format!("{doc_id}.md"));
    let output_witness = ValidatedWrite::new(&run.pair.target_folder, &output_path)?;
    let observed_output = hash_output(&run.pair.target_folder, &output_path)?;
    let pending = existing.as_ref().and_then(|entry| entry.pending.as_ref());
    if pending.is_none()
        && observed_output.as_ref().is_some_and(|hash| {
            existing
                .as_ref()
                .and_then(|entry| entry.committed.as_ref())
                .is_none_or(|generation| &generation.output_hash != hash)
        })
    {
        return Err(AppError::new("output_conflict"));
    }
    let redacted_at = pending.map_or_else(now, |pending| pending.review.redacted_at.clone());
    let request = ProcessRequest {
        sync_pair_id: run.pair.id,
        doc_id,
        source_hash_sha256: source_hash.clone(),
        processing_revision: run.pair.processing_revision,
        redacted_at,
        source_path: source_path
            .to_str()
            .ok_or_else(|| AppError::new("invalid_path"))?
            .into(),
    };
    let result: ProcessResult = if let Some(pending) = pending {
        let review = &pending.review;
        if review.source_hash != *source_hash
            || review.revision != run.pair.processing_revision
            || &review.engine != engine
        {
            return Err(AppError::new("recovery_pending"));
        }
        sidecar
            .request(
                "render_review",
                &ReviewRequest {
                    sync_pair_id: request.sync_pair_id,
                    doc_id: request.doc_id.clone(),
                    source_hash_sha256: request.source_hash_sha256.clone(),
                    processing_revision: request.processing_revision,
                    redacted_at: request.redacted_at.clone(),
                    source_path: request.source_path.clone(),
                    detections: review.detections.clone(),
                    decisions: review.decisions.clone(),
                    review_status: review.status,
                    reviewed_at: review.reviewed_at.clone(),
                    acknowledged_warnings: review.acknowledged_warnings.clone(),
                },
                DOCUMENT_TIMEOUT,
            )
            .await?
    } else {
        sidecar
            .request("process_document", &request, DOCUMENT_TIMEOUT)
            .await?
    };
    if result.sync_pair_id != request.sync_pair_id
        || result.doc_id != request.doc_id
        || result.processing_revision != request.processing_revision
        || result.source_hash_sha256 != request.source_hash_sha256
        || !same_timestamp(&result.redacted_at, &request.redacted_at)
        || &result.engine != engine
        || result
            .detections
            .iter()
            .any(|span| span.end > result.original_text.chars().count() as u64)
        || result
            .redactions
            .iter()
            .any(|span| span.end_offset > result.body.chars().count() as u64)
    {
        return Err(AppError::new("invalid_sidecar_response"));
    }
    source_witness
        .validate()
        .map_err(|_| AppError::new("source_changed"))?;
    if hash_output(&run.pair.source_folder, &source_path)
        .map_err(|_| AppError::new("unreadable_document"))?
        .as_ref()
        != Some(source_hash)
    {
        return Err(AppError::new("source_changed"));
    }
    output_witness.validate()?;
    let warned = !result.warnings.is_empty();
    let review = if let Some(pending) = pending {
        pending.review.clone()
    } else {
        if !matches!(
            result.review_status,
            ReviewStatus::Pending | ReviewStatus::NeedsRework
        ) {
            return Err(AppError::new("invalid_sidecar_response"));
        }
        ReviewRecord {
            schema_version: 1,
            key: key.clone(),
            source_hash: source_hash.clone(),
            revision: run.pair.processing_revision,
            detections: result.detections,
            decisions: Decisions::default(),
            status: result.review_status,
            notes: String::new(),
            acknowledged_warnings: vec![],
            warnings: result.warnings,
            redacted_at: request.redacted_at,
            reviewed_at: None,
            engine: result.engine,
            output_hash: format!("{:x}", Sha256::digest(result.markdown.as_bytes())),
        }
    };
    let candidate = CommitCandidate {
        key,
        source_hash: source_hash.clone(),
        revision: run.pair.processing_revision,
        markdown: result.markdown.into_bytes(),
        review,
    };
    let result = commit_generation(
        &run.pair,
        mapping,
        candidate,
        observed_output.as_deref(),
        &run.guard.source,
        &run.controller.inner.settings_path,
        &run.guard.config,
    );
    if result.is_err() {
        // Reconcile disk truth after indeterminate writes; the original safe error
        // remains visible even if reconciliation completed the generation.
        let _ = recover_pending(
            &run.pair,
            mapping,
            &run.guard.source,
            &run.controller.inner.settings_path,
            &run.guard.config,
        );
    }
    result?;
    Ok(Some(warned))
}

impl RunCounts {
    /// Invalid overflowing states saturate; validate with `is_consistent` before publication.
    pub fn completed(&self) -> u64 {
        self.processed
            .saturating_add(self.skipped)
            .saturating_add(self.failed)
    }

    pub fn is_consistent(&self) -> bool {
        self.warned <= self.processed
            && self
                .processed
                .checked_add(self.skipped)
                .and_then(|completed| completed.checked_add(self.failed))
                .and_then(|completed| completed.checked_add(self.unprocessed))
                == Some(self.discovered)
    }
}

/// P3.5 owns applying validated configuration/fingerprint changes under the held
/// guards before this snapshot is configured again and discovery begins.
pub async fn configure_pair(
    pair: &SyncPair,
    sidecar: &Sidecar,
) -> Result<ConfigureResult, AppError> {
    let configured: ConfigureResult = sidecar
        .request("configure", &configuration(pair), INITIALIZATION_TIMEOUT)
        .await?;
    if configured.sync_pair_id != pair.id
        || configured.processing_revision != pair.processing_revision
        || configured.engine.model_name != pair.config.model
    {
        return Err(AppError::new("invalid_sidecar_response"));
    }
    if pair
        .processing_fingerprint
        .as_ref()
        .is_some_and(|saved| saved != &fingerprint(&configured.engine))
    {
        return Err(AppError::new("processing_version_changed"));
    }
    Ok(configured)
}
fn fingerprint(engine: &EngineInfo) -> ProcessingFingerprint {
    ProcessingFingerprint {
        engine_version: engine.engine_version.clone(),
        extraction_version: engine.extraction_version.clone(),
        model_name: engine.model_name.clone(),
        model_version: engine.model_version.clone(),
    }
}

fn same_timestamp(left: &str, right: &str) -> bool {
    matches!((OffsetDateTime::parse(left, &Rfc3339), OffsetDateTime::parse(right, &Rfc3339)), (Ok(a), Ok(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, fs, sync::mpsc};

    thread_local! {
        static SCAN_PAUSE: RefCell<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>> = const { RefCell::new(None) };
    }

    pub(super) fn pause_scan() {
        SCAN_PAUSE.with_borrow_mut(|pause| {
            if let Some((ready, resume)) = pause.take() {
                ready.send(()).unwrap();
                resume.recv().unwrap();
            }
        });
    }

    #[test]
    fn standalone_scan_holds_app_config_and_source_guards_until_completion() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        let mut settings = super::super::settings::Settings::default();
        for name in ["first", "second"] {
            let source = root.path().join(format!("{name}-source"));
            let target = root.path().join(format!("{name}-target"));
            fs::create_dir(&source).unwrap();
            fs::create_dir(&target).unwrap();
            fs::write(source.join("document.docx"), b"synthetic").unwrap();
            settings.add(name, &source, &target).unwrap();
        }
        let path = config.join("settings.json");
        super::super::settings::save_settings(&path, &settings).unwrap();
        let controller = RunController::new(path);
        let worker = controller.clone();
        let pair_id = settings.sync_pairs[0].id;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let scan = std::thread::spawn(move || {
            SCAN_PAUSE.with_borrow_mut(|pause| *pause = Some((ready_tx, resume_rx)));
            worker.scan(pair_id)
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        // Pair mutations use this same nonblocking app operation acquisition.
        let mutation_error = controller.try_operation().err().map(|error| error.code);
        let other_error = controller
            .prepare(settings.sync_pairs[1].id, None, vec![])
            .err()
            .map(|error| error.code);
        let config_busy = CollectionGuard::acquire(&config).is_err();
        let source_busy = CollectionGuard::acquire(&settings.sync_pairs[0].source_folder).is_err();
        resume_tx.send(()).unwrap();
        assert_eq!(scan.join().unwrap().unwrap().files.len(), 1);
        assert_eq!(mutation_error.as_deref(), Some("operation_busy"));
        assert_eq!(other_error.as_deref(), Some("operation_busy"));
        assert!(config_busy);
        assert!(source_busy);
        assert!(controller.try_operation().is_ok());
        assert!(controller
            .prepare(settings.sync_pairs[1].id, None, vec![])
            .is_ok());
    }

    #[test]
    fn request_timestamps_preserve_python_microseconds_and_compare_exact_instants() {
        for (input, echoed) in [
            ("2026-09-19T12:00:00Z", "2026-09-19T12:00:00+00:00"),
            (
                "2026-09-19T12:00:00.123456789Z",
                "2026-09-19T12:00:00.123456Z",
            ),
            (
                "2026-09-19T12:00:00.100000Z",
                "2026-09-19T12:00:00.100000+00:00",
            ),
        ] {
            let timestamp = request_timestamp(OffsetDateTime::parse(input, &Rfc3339).unwrap());
            assert!(same_timestamp(&timestamp, echoed));
        }
        assert!(!same_timestamp(
            "2026-09-19T12:00:00.123457Z",
            "2026-09-19T12:00:00.123456Z"
        ));
        assert!(!same_timestamp("invalid", "invalid"));
    }
}
