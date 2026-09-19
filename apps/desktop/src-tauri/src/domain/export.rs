use super::paths::{canonical_directory, ensure_separate, reject_links};
use super::{
    audit::{append_audit, AuditAction, AuditEntry},
    mapping::{document_number, Mapping},
    review::{self, ApprovalBinding},
    settings::{load_settings, SyncPair},
    storage::{open_validated_read, ValidatedWrite},
    sync::{now, OperationGuard, RunController, RunCounts, RunOutcome},
};
use crate::error::AppError;
use crate::{
    protocol::{DocumentKey, EngineInfo, ReviewStatus},
    sidecar::Sidecar,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const LOCK_FILE: &str = ".redactio-export.lock";

#[derive(Debug, Serialize)]
pub struct ExportFailure {
    pub doc_id: String,
    pub error: AppError,
}

#[derive(Debug, Serialize)]
pub struct ExportSummary {
    pub sync_pair_id: Uuid,
    pub exported: Vec<String>,
    pub failed: Vec<ExportFailure>,
    pub error: Option<AppError>,
    pub cancelled: bool,
    pub audit_warning: bool,
}

struct Destination {
    path: PathBuf,
    witness: Option<ValidatedWrite>,
}

impl Destination {
    fn acquire(path: &Path, roots: &[PathBuf], config: &Path) -> Result<Self, AppError> {
        let create = ValidatedWrite::new(path, &path.join(LOCK_FILE))?;
        let path = validate_export_destination(path, roots, config)?;
        #[cfg(test)]
        tests::after_validation(&path);
        let marker = path.join(LOCK_FILE);
        let owner = Uuid::new_v4().to_string();
        create.create_atomic(owner.as_bytes())?;
        // Never adopt another owner after publication; retain before inspecting bytes.
        let witness = ValidatedWrite::new(&path, &marker)?;
        let mut bytes = Vec::new();
        open_validated_read(&path, &marker)?
            .take(37)
            .read_to_end(&mut bytes)?;
        witness.validate()?;
        if bytes != owner.as_bytes() {
            return Err(AppError::new("path_changed"));
        }
        #[cfg(test)]
        tests::after_lock(&path);
        let owned = Self {
            path,
            witness: Some(witness),
        };
        for entry in fs::read_dir(&owned.path)? {
            if entry?.file_name() != LOCK_FILE {
                return Err(AppError::new("export_not_empty"));
            }
        }
        owned.validate()?;
        Ok(owned)
    }

    fn validate(&self) -> Result<(), AppError> {
        self.witness.as_ref().unwrap().validate()
    }

    fn close(mut self) -> Result<(), AppError> {
        self.witness.take().unwrap().remove()
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        if let Some(witness) = self.witness.take() {
            let _ = witness.remove();
        }
    }
}

/// Resolve the sidecar lazily: cancelling the native picker needs no model/runtime.
pub async fn export_approved(
    controller: &RunController,
    pair_id: Uuid,
    doc_ids: Vec<String>,
    destination: Option<&Path>,
    sidecar: impl FnOnce() -> Result<Sidecar, AppError>,
) -> Result<ExportSummary, AppError> {
    let (mut pair, guard) = controller.lock_pair(pair_id)?;
    let settings_path = controller.settings_path();
    let config_dir = settings_path
        .parent()
        .ok_or_else(|| AppError::new("invalid_path"))?;
    let started_at = now();
    let mut summary = ExportSummary {
        sync_pair_id: pair_id,
        exported: vec![],
        failed: vec![],
        error: None,
        cancelled: destination.is_none(),
        audit_warning: false,
    };
    let result = async {
        let Some(destination) = destination else {
            return Ok(());
        };
        let mut unique = HashSet::new();
        if doc_ids.is_empty()
            || doc_ids
                .iter()
                .any(|id| document_number(id).is_err() || !unique.insert(id))
        {
            return Err(AppError::new("invalid_export_selection"));
        }
        let sidecar = sidecar()?;
        let config = pair.config.clone();
        let configured = super::detection::apply_configuration(
            settings_path,
            &guard,
            &mut pair,
            config,
            &sidecar,
        )
        .await?;
        let mapping = Mapping::load(&pair.source_folder, pair.id, &pair.target_folder)?;
        if doc_ids
            .iter()
            .any(|id| !mapping.entries().iter().any(|entry| entry.doc_id == *id))
        {
            return Err(AppError::new("unknown_document"));
        }
        let settings = load_settings(settings_path)?;
        let roots: Vec<_> = settings
            .sync_pairs
            .iter()
            .flat_map(|pair| [pair.source_folder.clone(), pair.target_folder.clone()])
            .collect();
        let owned = Destination::acquire(destination, &roots, config_dir)?;
        for doc_id in &doc_ids {
            #[cfg(test)]
            tests::before_file(doc_id, &owned.path);
            let key = DocumentKey {
                sync_pair_id: pair_id,
                doc_id: doc_id.clone(),
            };
            match export_one(
                &pair,
                &key,
                settings_path,
                &guard,
                &owned,
                &configured.engine,
            ) {
                Ok(()) => summary.exported.push(doc_id.clone()),
                Err(error) => summary.failed.push(ExportFailure {
                    doc_id: doc_id.clone(),
                    error,
                }),
            }
        }
        owned.close()
    }
    .await;
    summary.error = result.err();
    let counts = RunCounts {
        discovered: doc_ids.len() as u64,
        processed: summary.exported.len() as u64,
        failed: summary.failed.len() as u64,
        unprocessed: (doc_ids.len() - summary.exported.len() - summary.failed.len()) as u64,
        ..RunCounts::default()
    };
    let outcome = if summary.cancelled {
        RunOutcome::Cancelled
    } else if summary.error.is_some() {
        RunOutcome::Failed
    } else if !summary.failed.is_empty() {
        RunOutcome::CompletedWithErrors
    } else {
        RunOutcome::Completed
    };
    let mut error_codes: Vec<_> = summary
        .failed
        .iter()
        .map(|failure| failure.error.code.clone())
        .collect();
    error_codes.extend(summary.error.iter().map(|error| error.code.clone()));
    error_codes.sort();
    error_codes.dedup();
    summary.audit_warning = append_audit(
        config_dir,
        &AuditEntry {
            schema_version: 1,
            sync_pair_id: pair_id,
            run_id: Uuid::new_v4(),
            action: AuditAction::Export,
            started_at,
            finished_at: now(),
            outcome,
            counts,
            processing_revision: pair.processing_revision,
            engine: pair.processing_fingerprint.clone(),
            error_codes,
        },
    )
    .is_err();
    Ok(summary)
}

fn export_one(
    pair: &SyncPair,
    key: &DocumentKey,
    settings_path: &Path,
    guard: &OperationGuard,
    destination: &Destination,
    engine: &EngineInfo,
) -> Result<(), AppError> {
    destination.validate()?;
    // Capture destination absence/identity before reading the approval snapshot.
    let checked = ValidatedWrite::new(
        &destination.path,
        &destination.path.join(format!("{}.md", key.doc_id)),
    )?;
    let current = review::load_current(pair, key, settings_path, &guard.config, &guard.source)?;
    if &current.record.engine != engine {
        return Err(AppError::new("reprocess_required"));
    }
    if current.record.status != ReviewStatus::Approved {
        return Err(AppError::new("approval_required"));
    }
    let mut bytes = Vec::new();
    open_validated_read(
        &pair.target_folder,
        &pair.target_folder.join(format!("{}.md", key.doc_id)),
    )?
    .take(64 * 1024 * 1024 + 1)
    .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(AppError::new("output_too_large"));
    }
    let saved = ApprovalBinding {
        key: current.record.key.clone(),
        source_hash: current.record.source_hash.clone(),
        revision: current.record.revision.to_string(),
        output_hash: current.record.output_hash.clone(),
    };
    let observed = ApprovalBinding {
        key: key.clone(),
        source_hash: current.record.source_hash.clone(),
        revision: pair.processing_revision.to_string(),
        output_hash: format!("{:x}", Sha256::digest(&bytes)),
    };
    if !review::is_current_approval(&saved, &observed) {
        return Err(AppError::new("review_mismatch"));
    }
    #[cfg(test)]
    tests::after_snapshot();
    review::recheck(
        &current,
        pair,
        key,
        settings_path,
        &guard.config,
        &guard.source,
    )?;
    destination.validate()?;
    checked.create_atomic(&bytes)
}

/// Supply both roots of every saved pair; unavailable roots fail closed.
pub fn validate_export_destination(
    destination: &Path,
    roots: &[PathBuf],
    config_dir: &Path,
) -> Result<PathBuf, AppError> {
    let canonical = canonical_directory(destination)?;
    reject_links(destination)?;
    for root in roots.iter().map(PathBuf::as_path).chain([config_dir]) {
        ensure_separate(&canonical, &canonical_directory(root)?)?;
    }
    if fs::read_dir(&canonical)?.next().transpose()?.is_some() {
        return Err(AppError::new("export_not_empty"));
    }
    Ok(canonical)
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
