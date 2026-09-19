use super::{
    mapping::{
        classify, commit_generation, recover_pending, validate_context, CollectionGuard,
        CommitCandidate, DocumentState, Mapping, ReviewRecord,
    },
    scan::hash_output,
    settings::SyncPair,
    storage::ValidatedWrite,
    sync::{now, same_timestamp, RunController},
};
use crate::{
    error::AppError,
    protocol::{
        Decisions, Detection, DocumentKey, EngineInfo, OutputEntry, ProcessResult, ReviewRequest,
        ReviewStatus,
    },
    sidecar::{Sidecar, DOCUMENT_TIMEOUT},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalBinding {
    pub key: DocumentKey,
    pub source_hash: String,
    pub revision: String,
    pub output_hash: String,
}

/// The caller must verify approved status and current disk bindings first.
pub fn is_current_approval(saved: &ApprovalBinding, current: &ApprovalBinding) -> bool {
    saved == current
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewViewData {
    pub key: DocumentKey,
    pub source_hash: String,
    pub revision: uuid::Uuid,
    pub expected_output_hash: String,
    pub original_text: String,
    pub markdown: String,
    pub body: String,
    pub detections: Vec<Detection>,
    pub redactions: Vec<OutputEntry>,
    pub decisions: Decisions,
    pub warnings: Vec<String>,
    pub acknowledged_warnings: Vec<String>,
    pub notes: String,
    pub status: ReviewStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveReview {
    pub expected_output_hash: String,
    pub decisions: Decisions,
    pub status: ReviewStatus,
    pub notes: String,
    pub acknowledged_warnings: Vec<String>,
}

pub(crate) struct CurrentReview {
    mapping: Mapping,
    pub(crate) record: ReviewRecord,
    source: ValidatedWrite,
    output: ValidatedWrite,
    source_path: String,
}

/// Direct document commands refresh the saved engine binding even if the pair
/// has not been activated or scanned in this app session.
pub async fn open_configured(
    controller: &RunController,
    sidecar: &Sidecar,
    key: &DocumentKey,
) -> Result<ReviewViewData, AppError> {
    let (mut pair, guard) = controller.lock_pair(key.sync_pair_id)?;
    let config = pair.config.clone();
    let configured = super::detection::apply_configuration(
        controller.settings_path(),
        &guard,
        &mut pair,
        config,
        sidecar,
    )
    .await?;
    open_review(
        &pair,
        key,
        controller.settings_path(),
        &guard.config,
        &guard.source,
        sidecar,
        &configured.engine,
    )
    .await
}

pub async fn save_configured(
    controller: &RunController,
    sidecar: &Sidecar,
    key: &DocumentKey,
    input: SaveReview,
) -> Result<ReviewViewData, AppError> {
    let (mut pair, guard) = controller.lock_pair(key.sync_pair_id)?;
    let config = pair.config.clone();
    let configured = super::detection::apply_configuration(
        controller.settings_path(),
        &guard,
        &mut pair,
        config,
        sidecar,
    )
    .await?;
    save_review(
        &pair,
        key,
        input,
        controller.settings_path(),
        &guard.config,
        &guard.source,
        sidecar,
        &configured.engine,
    )
    .await
}

/// Caller retains the app operation lock and config → source guards and configures
/// the current saved pair before entry. Original text only lives in this response.
pub async fn open_review(
    pair: &SyncPair,
    key: &DocumentKey,
    settings_path: &Path,
    config_guard: &CollectionGuard,
    source_guard: &CollectionGuard,
    sidecar: &Sidecar,
    engine: &EngineInfo,
) -> Result<ReviewViewData, AppError> {
    let current = load_current(pair, key, settings_path, config_guard, source_guard)?;
    let result = render(&current, &current.record, sidecar, engine).await?;
    if digest(result.markdown.as_bytes()) != current.record.output_hash {
        return Err(AppError::new("review_mismatch"));
    }
    recheck(
        &current,
        pair,
        key,
        settings_path,
        config_guard,
        source_guard,
    )?;
    Ok(view(current.record, result))
}

/// The journal requires these same retained guards and actual settings path.
#[allow(clippy::too_many_arguments)]
pub async fn save_review(
    pair: &SyncPair,
    key: &DocumentKey,
    input: SaveReview,
    settings_path: &Path,
    config_guard: &CollectionGuard,
    source_guard: &CollectionGuard,
    sidecar: &Sidecar,
    engine: &EngineInfo,
) -> Result<ReviewViewData, AppError> {
    let mut current = load_current(pair, key, settings_path, config_guard, source_guard)?;
    if input.expected_output_hash != current.record.output_hash {
        return Err(AppError::new("output_conflict"));
    }
    let existing = render(&current, &current.record, sidecar, engine).await?;
    if digest(existing.markdown.as_bytes()) != current.record.output_hash {
        return Err(AppError::new("review_mismatch"));
    }
    let changed = input.decisions != current.record.decisions;
    let mut record = current.record.clone();
    record.decisions = input.decisions;
    record.notes = input.notes;
    record.acknowledged_warnings = input.acknowledged_warnings;
    let timestamp = now();
    record.status = if changed {
        if record.warnings.is_empty() {
            ReviewStatus::Pending
        } else {
            ReviewStatus::NeedsRework
        }
    } else {
        input.status
    };
    if record.status == ReviewStatus::Approved
        && (existing.body_was_empty
            || record
                .warnings
                .iter()
                .any(|w| !record.acknowledged_warnings.contains(w)))
    {
        return Err(AppError::new("approval_not_allowed"));
    }
    if existing.body_was_empty
        || (record.status == ReviewStatus::Pending && !record.warnings.is_empty())
    {
        record.status = ReviewStatus::NeedsRework;
    }
    if changed {
        record.redacted_at = timestamp.clone();
    }
    record.reviewed_at = matches!(
        record.status,
        ReviewStatus::Approved | ReviewStatus::Rejected
    )
    .then_some(timestamp);
    validate_spans(&record, existing.original_text.chars().count() as u64)?;
    let result = render(&current, &record, sidecar, engine).await?;
    record.output_hash = digest(result.markdown.as_bytes());
    recheck(
        &current,
        pair,
        key,
        settings_path,
        config_guard,
        source_guard,
    )?;
    let candidate = CommitCandidate {
        key: key.clone(),
        source_hash: record.source_hash.clone(),
        revision: record.revision,
        markdown: result.markdown.as_bytes().to_vec(),
        review: record.clone(),
    };
    let committed = commit_generation(
        pair,
        &mut current.mapping,
        candidate,
        Some(&current.record.output_hash),
        source_guard,
        settings_path,
        config_guard,
    );
    if committed.is_err() {
        // Reconcile any partial publication without hiding the original safe error.
        let _ = recover_pending(
            pair,
            &mut current.mapping,
            source_guard,
            settings_path,
            config_guard,
        );
    }
    committed?;
    Ok(view(record, result))
}

pub(crate) fn load_current(
    pair: &SyncPair,
    key: &DocumentKey,
    settings_path: &Path,
    config_guard: &CollectionGuard,
    source_guard: &CollectionGuard,
) -> Result<CurrentReview, AppError> {
    if key.sync_pair_id != pair.id {
        return Err(AppError::new("mapping_pair_mismatch"));
    }
    super::mapping::document_number(&key.doc_id)?;
    let mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder)?;
    validate_context(
        pair,
        &mapping,
        source_guard,
        settings_path,
        config_guard,
        true,
    )
    .map_err(|e| {
        if e.code == "revision_changed" {
            AppError::new("reprocess_required")
        } else {
            e
        }
    })?;
    let entry = mapping
        .entries()
        .iter()
        .find(|entry| entry.doc_id == key.doc_id)
        .ok_or_else(|| AppError::new("unknown_document"))?;
    if entry.pending.is_some() {
        return Err(AppError::new("recovery_pending"));
    }
    let source_path = pair.source_folder.join(&entry.relative_path);
    let output_path = pair.target_folder.join(format!("{}.md", key.doc_id));
    let source = ValidatedWrite::new(&pair.source_folder, &source_path)?;
    let output = ValidatedWrite::new(&pair.target_folder, &output_path)?;
    let source_hash = hash_output(&pair.source_folder, &source_path)?;
    let output_hash = hash_output(&pair.target_folder, &output_path)?;
    match classify(
        source_hash.as_deref(),
        output_hash.as_deref(),
        &pair.processing_revision.to_string(),
        entry.committed.as_ref(),
        false,
    ) {
        DocumentState::Current => (),
        DocumentState::MissingSource => return Err(AppError::new("missing_source")),
        DocumentState::Conflict => return Err(AppError::new("output_conflict")),
        DocumentState::RecoveryPending => return Err(AppError::new("recovery_pending")),
        _ => return Err(AppError::new("reprocess_required")),
    }
    let record = ReviewRecord::read(&pair.source_folder, key, entry.committed.as_ref().unwrap())?;
    validate_spans(&record, 1_000_000)?;
    source.validate()?;
    output.validate()?;
    Ok(CurrentReview {
        mapping,
        record,
        source,
        output,
        source_path: source_path
            .to_str()
            .ok_or_else(|| AppError::new("invalid_path"))?
            .into(),
    })
}

pub(crate) fn recheck(
    current: &CurrentReview,
    pair: &SyncPair,
    key: &DocumentKey,
    settings_path: &Path,
    config_guard: &CollectionGuard,
    source_guard: &CollectionGuard,
) -> Result<(), AppError> {
    current.source.validate()?;
    current.output.validate()?;
    let observed = load_current(pair, key, settings_path, config_guard, source_guard)?;
    if observed.mapping.entries() != current.mapping.entries() || observed.record != current.record
    {
        return Err(AppError::new("review_mismatch"));
    }
    Ok(())
}

fn validate_spans(record: &ReviewRecord, text_length: u64) -> Result<(), AppError> {
    record.validate()?;
    if record
        .detections
        .iter()
        .chain(&record.decisions.manual)
        .any(|span| span.end > text_length)
    {
        return Err(AppError::new("invalid_review"));
    }
    Ok(())
}

async fn render(
    current: &CurrentReview,
    record: &ReviewRecord,
    sidecar: &Sidecar,
    engine: &EngineInfo,
) -> Result<ProcessResult, AppError> {
    if &record.engine != engine {
        return Err(AppError::new("reprocess_required"));
    }
    let request = ReviewRequest {
        sync_pair_id: record.key.sync_pair_id,
        doc_id: record.key.doc_id.clone(),
        source_hash_sha256: record.source_hash.clone(),
        processing_revision: record.revision,
        redacted_at: record.redacted_at.clone(),
        source_path: current.source_path.clone(),
        detections: record.detections.clone(),
        decisions: record.decisions.clone(),
        review_status: record.status,
        reviewed_at: record.reviewed_at.clone(),
        acknowledged_warnings: record.acknowledged_warnings.clone(),
    };
    let result: ProcessResult = sidecar
        .request("render_review", &request, DOCUMENT_TIMEOUT)
        .await?;
    validate_spans(record, result.original_text.chars().count() as u64)?;
    let body_length = result.body.chars().count() as u64;
    if result.sync_pair_id != record.key.sync_pair_id
        || result.doc_id != record.key.doc_id
        || result.source_hash_sha256 != record.source_hash
        || result.processing_revision != record.revision
        || !same_timestamp(&result.redacted_at, &record.redacted_at)
        || result.engine != record.engine
        || result.detections != record.detections
        || result.warnings != record.warnings
        || result.review_status != record.status
        || result.body_was_empty != result.original_text.trim().is_empty()
        || (result.body_was_empty && result.review_status != ReviewStatus::NeedsRework)
        || result
            .redactions
            .iter()
            .any(|span| span.end_offset > body_length)
    {
        return Err(AppError::new("invalid_sidecar_response"));
    }
    Ok(result)
}

fn view(record: ReviewRecord, result: ProcessResult) -> ReviewViewData {
    ReviewViewData {
        key: record.key,
        source_hash: record.source_hash,
        revision: record.revision,
        expected_output_hash: record.output_hash,
        original_text: result.original_text,
        markdown: result.markdown,
        body: result.body,
        detections: record.detections,
        redactions: result.redactions,
        decisions: record.decisions,
        warnings: record.warnings,
        acknowledged_warnings: record.acknowledged_warnings,
        notes: record.notes,
        status: record.status,
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
