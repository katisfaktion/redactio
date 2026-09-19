use super::{
    settings::ProcessingFingerprint,
    storage::ValidatedWrite,
    sync::{RunCounts, RunOutcome},
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::Path,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

pub const AUDIT_FILE: &str = "audit-log.jsonl";
const MAX_RECORD: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditAction {
    Sync,
    Export,
    Recovery,
}

/// Deliberate whitelist: no UI names, filenames, paths, content, notes, or rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEntry {
    pub schema_version: u8,
    #[serde(deserialize_with = "crate::protocol::canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "crate::protocol::canonical_uuid")]
    pub run_id: Uuid,
    pub action: AuditAction,
    pub started_at: String,
    pub finished_at: String,
    pub outcome: RunOutcome,
    pub counts: RunCounts,
    #[serde(deserialize_with = "crate::protocol::canonical_uuid")]
    pub processing_revision: Uuid,
    pub engine: Option<ProcessingFingerprint>,
    pub error_codes: Vec<String>,
}

impl AuditEntry {
    fn validate(&self) -> Result<(), AppError> {
        let start = OffsetDateTime::parse(&self.started_at, &Rfc3339)
            .map_err(|_| AppError::new("invalid_audit"))?;
        let end = OffsetDateTime::parse(&self.finished_at, &Rfc3339)
            .map_err(|_| AppError::new("invalid_audit"))?;
        if self.schema_version != 1
            || self.sync_pair_id.is_nil()
            || self.run_id.is_nil()
            || self.processing_revision.is_nil()
            || end < start
            || !self.counts.is_consistent()
        {
            return Err(AppError::new("invalid_audit"));
        }
        for code in &self.error_codes {
            serde_json::from_value::<AppError>(
                serde_json::json!({"code": code, "retryable": false}),
            )
            .map_err(|_| AppError::new("invalid_audit"))?;
        }
        Ok(())
    }
}

pub fn append_audit(config_dir: &Path, entry: &AuditEntry) -> Result<(), AppError> {
    entry.validate()?;
    let path = config_dir.join(AUDIT_FILE);
    if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        match ValidatedWrite::new(config_dir, &path)?.create_atomic(b"") {
            Ok(()) => (),
            Err(error) if matches!(error.code.as_str(), "path_exists" | "path_changed") => (),
            Err(error) => return Err(error),
        }
    }
    let mut file = ValidatedWrite::new(config_dir, &path)?.open_update()?;
    file.lock().map_err(|_| AppError::new("audit_busy"))?;
    // Lock the stable log inode; appends never replace or remove this file.
    let mut reader = BufReader::new(&mut file);
    let mut boundary = 0u64;
    let recovered = loop {
        let mut line = Vec::new();
        Read::by_ref(&mut reader)
            .take(MAX_RECORD + 1)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            break false;
        }
        if line.len() as u64 > MAX_RECORD {
            return Err(AppError::new("invalid_audit"));
        }
        if line.last() != Some(&b'\n') {
            break true;
        }
        let previous: AuditEntry =
            serde_json::from_slice(&line).map_err(|_| AppError::new("invalid_audit"))?;
        previous.validate()?;
        boundary += line.len() as u64;
    };
    drop(reader);
    let mut bytes = Vec::new();
    if recovered {
        let mut recovery = entry.clone();
        recovery.action = AuditAction::Recovery;
        recovery.counts = RunCounts::default();
        recovery.error_codes = vec!["audit_truncated_record".into()];
        serde_json::to_writer(&mut bytes, &recovery).map_err(|_| AppError::new("invalid_audit"))?;
        bytes.push(b'\n');
    }
    let record = serde_json::to_vec(entry).map_err(|_| AppError::new("invalid_audit"))?;
    if record.len() as u64 >= MAX_RECORD {
        return Err(AppError::new("invalid_audit"));
    }
    bytes.extend(record);
    bytes.push(b'\n');
    if bytes
        .split_inclusive(|byte| *byte == b'\n')
        .any(|line| line.len() as u64 > MAX_RECORD)
    {
        return Err(AppError::new("invalid_audit"));
    }
    if recovered {
        file.set_len(boundary)?;
    }
    file.seek(SeekFrom::End(0))?;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        file.write_all(line)?;
        file.flush()?;
        file.sync_all()?;
    }
    Ok(())
}
