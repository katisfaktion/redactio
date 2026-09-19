macro_rules! checkpoint {
    ($stage:ident) => {
        #[cfg(test)]
        super::recovery_tests::checkpoint(super::recovery_tests::CommitStage::$stage)?;
    };
}

use super::*;
use crate::{
    domain::{
        scan::hash_output,
        settings::{Settings, SyncPair},
    },
    protocol::{Decisions, Detection, DetectionOrigin, DocumentKey, EngineInfo, ReviewStatus},
};
use sha2::{Digest, Sha256};

const MAX_RECORD: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRecord {
    pub schema_version: u8,
    pub key: DocumentKey,
    #[serde(deserialize_with = "crate::protocol::sha256")]
    pub source_hash: String,
    #[serde(deserialize_with = "crate::protocol::canonical_uuid")]
    pub revision: Uuid,
    pub detections: Vec<Detection>,
    pub decisions: Decisions,
    pub status: ReviewStatus,
    pub notes: String,
    #[serde(deserialize_with = "crate::protocol::safe_codes")]
    pub acknowledged_warnings: Vec<String>,
    #[serde(deserialize_with = "crate::protocol::safe_codes")]
    pub warnings: Vec<String>,
    #[serde(deserialize_with = "crate::protocol::timestamp")]
    pub redacted_at: String,
    #[serde(deserialize_with = "crate::protocol::optional_timestamp")]
    pub reviewed_at: Option<String>,
    pub engine: EngineInfo,
    #[serde(deserialize_with = "crate::protocol::sha256")]
    pub output_hash: String,
}

impl ReviewRecord {
    pub fn validate(&self) -> Result<(), AppError> {
        // The wire parser also validates values assembled in Rust before persistence.
        let bytes = serde_json::to_vec(self).map_err(|_| AppError::new("invalid_review"))?;
        serde_json::from_slice::<Self>(&bytes).map_err(|_| AppError::new("invalid_review"))?;
        let redacted = timestamp(&self.redacted_at)?;
        if self.schema_version != 1
            || self.revision.is_nil()
            || !valid_hash(&self.source_hash)
            || !valid_hash(&self.output_hash)
            || self
                .reviewed_at
                .as_deref()
                .map(timestamp)
                .transpose()?
                .is_some_and(|t| t < redacted)
            || (matches!(self.status, ReviewStatus::Approved | ReviewStatus::Rejected)
                && self.reviewed_at.is_none())
        {
            return Err(AppError::new("invalid_review"));
        }
        let mut ids = HashSet::new();
        for detection in &self.detections {
            if detection.origin != DetectionOrigin::Automatic || !ids.insert(&detection.id) {
                return Err(AppError::new("invalid_review"));
            }
        }
        let mut dismissed = HashSet::new();
        for id in &self.decisions.dismissed_ids {
            if !ids.contains(id) || !dismissed.insert(id) {
                return Err(AppError::new("invalid_review"));
            }
        }
        for detection in &self.decisions.manual {
            if detection.origin != DetectionOrigin::Manual
                || detection.confidence.is_some()
                || !ids.insert(&detection.id)
            {
                return Err(AppError::new("invalid_review"));
            }
        }
        let mut warnings = HashSet::new();
        for code in &self.warnings {
            if !warnings.insert(code) {
                return Err(AppError::new("invalid_review"));
            }
        }
        let mut acknowledged = HashSet::new();
        for code in &self.acknowledged_warnings {
            if !warnings.contains(code) || !acknowledged.insert(code) {
                return Err(AppError::new("invalid_review"));
            }
        }
        if self.status == ReviewStatus::Approved
            && (acknowledged != warnings
                || self.warnings.iter().any(|code| code == "empty_document"))
        {
            return Err(AppError::new("invalid_review"));
        }
        Ok(())
    }

    /// Reads the private, ID-derived location and binds the exact bytes to the
    /// successful generation. The caller still checks current source/revision
    /// and actual output, and rejects pending entries before treating approval as effective.
    pub fn read(
        source: &Path,
        key: &DocumentKey,
        generation: &Generation,
    ) -> Result<Self, AppError> {
        document_number(&key.doc_id)?;
        generation.validate()?;
        let bytes = read_bytes(source, &review_path(source, &key.doc_id))?;
        if digest(&bytes) != generation.review_hash {
            return Err(AppError::new("review_mismatch"));
        }
        let record: Self =
            serde_json::from_slice(&bytes).map_err(|_| AppError::new("invalid_review"))?;
        record.validate()?;
        if record.key != *key
            || record.source_hash != generation.source_hash
            || record.revision.to_string() != generation.revision
            || record.output_hash != generation.output_hash
        {
            return Err(AppError::new("review_mismatch"));
        }
        Ok(record)
    }
}

#[derive(Debug, Clone)]
pub struct CommitCandidate {
    pub key: DocumentKey,
    pub source_hash: String,
    pub revision: Uuid,
    pub markdown: Vec<u8>,
    pub review: ReviewRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCommit {
    pub generation: Generation,
    pub prior_output_hash: Option<String>,
    pub prior_review_hash: Option<String>,
    pub review: ReviewRecord,
    pub output_temporary: String,
    pub review_temporary: String,
}

impl PendingCommit {
    pub(super) fn validate(&self, pair_id: Uuid, entry: &MappingEntry) -> Result<(), AppError> {
        self.generation.validate()?;
        self.review.validate()?;
        for name in [&self.output_temporary, &self.review_temporary] {
            let id = name
                .strip_prefix(".redactio-commit-")
                .and_then(|s| s.strip_suffix(".tmp"))
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| !id.is_nil())
                .ok_or_else(|| AppError::new("invalid_mapping"))?;
            if *name != format!(".redactio-commit-{id}.tmp") {
                return Err(AppError::new("invalid_mapping"));
            }
        }
        if self.output_temporary == self.review_temporary
            || self.review.key.sync_pair_id != pair_id
            || self.review.key.doc_id != entry.doc_id
            || self.review.source_hash != self.generation.source_hash
            || self.review.revision.to_string() != self.generation.revision
            || self.review.output_hash != self.generation.output_hash
            || digest(&review_bytes(&self.review)?) != self.generation.review_hash
            || self.generation.last_processed_at
                != *self
                    .review
                    .reviewed_at
                    .as_ref()
                    .unwrap_or(&self.review.redacted_at)
            || self
                .prior_output_hash
                .as_ref()
                .is_some_and(|h| !valid_hash(h))
            || self
                .prior_review_hash
                .as_ref()
                .is_some_and(|h| !valid_hash(h))
            || self
                .prior_output_hash
                .as_ref()
                .is_some_and(|h| entry.committed.as_ref().is_none_or(|g| *h != g.output_hash))
            || self
                .prior_review_hash
                .as_ref()
                .is_some_and(|h| entry.committed.as_ref().is_none_or(|g| *h != g.review_hash))
            || entry
                .committed
                .as_ref()
                .is_some_and(|g| g.first_processed_at != self.generation.first_processed_at)
        {
            return Err(AppError::new("invalid_mapping"));
        }
        Ok(())
    }
}

/// Acquire config_guard, then guard, and retain both throughout the operation.
/// A candidate's key must already be durably reserved with reserve_with_guard.
/// expected_output_hash is required for an existing output (including review saves).
/// Policy confirmation for discarding reviewed work belongs to the host caller.
pub fn commit_generation(
    pair: &SyncPair,
    mapping: &mut Mapping,
    candidate: CommitCandidate,
    expected_output_hash: Option<&str>,
    guard: &CollectionGuard,
    settings_path: &Path,
    config_guard: &CollectionGuard,
) -> Result<Generation, AppError> {
    validate_context(pair, mapping, guard, settings_path, config_guard, true)?;
    candidate.review.validate()?;
    if candidate.key.sync_pair_id != pair.id
        || candidate.review.key != candidate.key
        || candidate.revision != pair.processing_revision
        || candidate.review.revision != candidate.revision
        || candidate.source_hash != candidate.review.source_hash
        || candidate.review.output_hash != digest(&candidate.markdown)
        || candidate.markdown.len() as u64 > MAX_RECORD
        || std::str::from_utf8(&candidate.markdown).is_err()
    {
        return Err(AppError::new("candidate_mismatch"));
    }
    reload(mapping)?;
    let index = entry_index(mapping, &candidate.key.doc_id)?;
    check_source(pair, &mapping.data.entries[index], &candidate.source_hash)?;
    checkpoint!(Reserved);
    ensure_reviews(&pair.source_folder)?;
    if mapping.data.entries[index].pending.is_some() {
        recover_one(pair, mapping, index, guard, settings_path, config_guard)?;
        if let Some(pending) = &mapping.data.entries[index].pending {
            // A retry may refill interrupted staging only with exactly the same candidate.
            if pending.review != candidate.review
                || pending.generation.output_hash != digest(&candidate.markdown)
            {
                return Err(AppError::new("recovery_pending"));
            }
        } else if let Some(generation) = &mapping.data.entries[index].committed {
            if generation.output_hash == candidate.review.output_hash
                && generation.review_hash == digest(&review_bytes(&candidate.review)?)
            {
                return Ok(generation.clone());
            }
        }
    }
    let output = pair
        .target_folder
        .join(format!("{}.md", candidate.key.doc_id));
    let review = review_path(&pair.source_folder, &candidate.key.doc_id);
    if mapping.data.entries[index].pending.is_none() {
        let output_write = ValidatedWrite::new(&pair.target_folder, &output)?;
        let review_write = ValidatedWrite::new(&pair.source_folder, &review)?;
        let prior_output = hash_output(&pair.target_folder, &output)?;
        let prior_review = hash_output(&pair.source_folder, &review)?;
        let old = mapping.data.entries[index].committed.as_ref();
        if prior_output.as_deref() != expected_output_hash
            || prior_output
                .as_ref()
                .is_some_and(|h| old.is_none_or(|g| g.output_hash != *h))
            || prior_review
                .as_ref()
                .is_some_and(|h| old.is_none_or(|g| g.review_hash != *h))
        {
            return Err(AppError::new("output_conflict"));
        }
        let last = candidate
            .review
            .reviewed_at
            .as_ref()
            .unwrap_or(&candidate.review.redacted_at)
            .clone();
        let generation = Generation {
            source_hash: candidate.source_hash,
            revision: candidate.revision.to_string(),
            output_hash: candidate.review.output_hash.clone(),
            review_hash: digest(&review_bytes(&candidate.review)?),
            first_processed_at: old.map_or_else(
                || candidate.review.redacted_at.clone(),
                |g| g.first_processed_at.clone(),
            ),
            last_processed_at: last,
        };
        let pending = PendingCommit {
            generation,
            prior_output_hash: prior_output,
            prior_review_hash: prior_review,
            review: candidate.review,
            output_temporary: temporary_name(),
            review_temporary: temporary_name(),
        };
        output_write.validate()?;
        review_write.validate()?;
        let mut data = mapping.data.clone();
        data.entries[index].pending = Some(pending);
        persist(mapping, data)?;
    }
    checkpoint!(Journaled);
    let pending = mapping.data.entries[index]
        .pending
        .clone()
        .ok_or_else(|| AppError::new("recovery_pending"))?;
    stage(
        &pair.target_folder,
        &pair.target_folder.join(&pending.output_temporary),
        &candidate.markdown,
    )?;
    checkpoint!(OutputStaged);
    stage(
        &pair.source_folder,
        &review.parent().unwrap().join(&pending.review_temporary),
        &review_bytes(&pending.review)?,
    )?;
    checkpoint!(ReviewStaged);
    validate_context(pair, mapping, guard, settings_path, config_guard, true)?;
    check_source(
        pair,
        &mapping.data.entries[index],
        &pending.generation.source_hash,
    )?;
    recover_one(pair, mapping, index, guard, settings_path, config_guard)?;
    mapping.data.entries[index]
        .committed
        .clone()
        .filter(|_| mapping.data.entries[index].pending.is_none())
        .ok_or_else(|| AppError::new("recovery_pending"))
}

/// Reconciles disk hashes, never journal stage labels. A partial candidate whose
/// staged bytes are missing stays pending for an exact retry; untouched attempts
/// can be cleared for regeneration while retaining their reservation/counter.
/// Recovered old generations retain their old source/revision binding (stale).
pub fn recover_pending(
    pair: &SyncPair,
    mapping: &mut Mapping,
    guard: &CollectionGuard,
    settings_path: &Path,
    config_guard: &CollectionGuard,
) -> Result<(), AppError> {
    validate_context(pair, mapping, guard, settings_path, config_guard, false)?;
    reload(mapping)?;
    for index in 0..mapping.data.entries.len() {
        recover_one(pair, mapping, index, guard, settings_path, config_guard)?;
    }
    Ok(())
}

fn recover_one(
    pair: &SyncPair,
    mapping: &mut Mapping,
    index: usize,
    guard: &CollectionGuard,
    settings_path: &Path,
    config_guard: &CollectionGuard,
) -> Result<(), AppError> {
    let Some(pending) = mapping.data.entries[index].pending.clone() else {
        return Ok(());
    };
    validate_context(pair, mapping, guard, settings_path, config_guard, false)?;
    let mapping_witness = ValidatedWrite::new(&mapping.source, &mapping.source.join(MAPPING_FILE))?;
    if MappingData::read(&mapping.source)? != mapping.data {
        return Err(AppError::new("mapping_changed"));
    }
    let output = pair
        .target_folder
        .join(format!("{}.md", mapping.data.entries[index].doc_id));
    let review = review_path(&pair.source_folder, &mapping.data.entries[index].doc_id);
    // Retain both destination witnesses throughout ownership inspection/publication.
    let output_write = ValidatedWrite::new(&pair.target_folder, &output)?;
    let review_write = ValidatedWrite::new(&pair.source_folder, &review)?;
    let output_hash = hash_output(&pair.target_folder, &output)?;
    let review_hash = hash_output(&pair.source_folder, &review)?;
    let output_done = output_hash.as_ref() == Some(&pending.generation.output_hash);
    let review_done = review_hash.as_ref() == Some(&pending.generation.review_hash);
    if (!output_done && output_hash != pending.prior_output_hash)
        || (!review_done && review_hash != pending.prior_review_hash)
    {
        return Err(AppError::new("output_conflict"));
    }
    let output_temp = pair.target_folder.join(&pending.output_temporary);
    let review_temp = review.parent().unwrap().join(&pending.review_temporary);
    let staged_output = staged_bytes(
        &pair.target_folder,
        &output_temp,
        &pending.generation.output_hash,
    )?;
    let staged_review = staged_bytes(
        &pair.source_folder,
        &review_temp,
        &pending.generation.review_hash,
    )?;
    if (!output_done && staged_output.is_none()) || (!review_done && staged_review.is_none()) {
        if !output_done && !review_done {
            cleanup(pair, &pending)?;
            let mut data = mapping.data.clone();
            data.entries[index].pending = None;
            persist(mapping, data)?;
        }
        return Ok(());
    }
    // Current disk source/revision are observed before finishing; they never
    // replace the candidate bindings. A recovered old approval remains stale.
    let _current_source = hash_output(
        &pair.source_folder,
        &pair
            .source_folder
            .join(&mapping.data.entries[index].relative_path),
    )?;
    mapping_witness.validate()?;
    output_write.validate()?;
    review_write.validate()?;
    if !output_done {
        output_write.write_atomic(staged_output.as_deref().unwrap())?;
    }
    checkpoint!(OutputReplaced);
    if !review_done {
        review_write.write_atomic(staged_review.as_deref().unwrap())?;
    }
    checkpoint!(ReviewReplaced);
    if hash_output(&pair.target_folder, &output)?.as_ref() != Some(&pending.generation.output_hash)
        || hash_output(&pair.source_folder, &review)?.as_ref()
            != Some(&pending.generation.review_hash)
    {
        return Err(AppError::new("output_conflict"));
    }
    cleanup(pair, &pending)?;
    let mut data = mapping.data.clone();
    data.entries[index].committed = Some(pending.generation);
    data.entries[index].pending = None;
    persist(mapping, data)
}

fn validate_context(
    pair: &SyncPair,
    mapping: &Mapping,
    guard: &CollectionGuard,
    settings_path: &Path,
    config_guard: &CollectionGuard,
    current: bool,
) -> Result<(), AppError> {
    let config = settings_path
        .parent()
        .ok_or_else(|| AppError::new("invalid_path"))?;
    config_guard.validate(config)?;
    guard.validate(&pair.source_folder)?;
    if mapping.pair_id != pair.id
        || !super::super::paths::same_directory(&mapping.source, &pair.source_folder)?
        || !super::super::paths::same_directory(&mapping.target, &pair.target_folder)?
    {
        return Err(AppError::new("mapping_pair_mismatch"));
    }
    super::super::recovery::ensure_recovered(&pair.source_folder, pair.id)?;
    let settings: Settings = serde_json::from_slice(&read_bytes(config, settings_path)?)
        .map_err(|_| AppError::new("invalid_settings"))?;
    settings.validate_roots_with(&[config.to_path_buf()])?;
    let saved = settings
        .sync_pairs
        .iter()
        .find(|p| p.id == pair.id)
        .ok_or_else(|| AppError::new("unknown_pair"))?;
    if !super::super::paths::same_directory(&saved.source_folder, &pair.source_folder)?
        || !super::super::paths::same_directory(&saved.target_folder, &pair.target_folder)?
    {
        return Err(AppError::new("mapping_pair_mismatch"));
    }
    if current
        && (saved.processing_revision != pair.processing_revision
            || saved.config != pair.config
            || saved.processing_fingerprint != pair.processing_fingerprint)
    {
        return Err(AppError::new("revision_changed"));
    }
    Ok(())
}

fn reload(mapping: &mut Mapping) -> Result<(), AppError> {
    let data = MappingData::read(&mapping.source)?;
    data.validate_binding(mapping.pair_id, &mapping.target)?;
    mapping.data = data;
    Ok(())
}

fn persist(mapping: &mut Mapping, data: MappingData) -> Result<(), AppError> {
    data.validate()?;
    let write = ValidatedWrite::new(&mapping.source, &mapping.source.join(MAPPING_FILE))?;
    if MappingData::read(&mapping.source)? != mapping.data {
        return Err(AppError::new("mapping_changed"));
    }
    let bytes = serde_json::to_vec_pretty(&data).map_err(|_| AppError::new("invalid_mapping"))?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(AppError::new("record_too_large"));
    }
    let result = write.write_atomic(&bytes);
    // Replacement may already be on disk when storage reports an error. Reload
    // actual truth, but preserve the original safe code and any native backups.
    let reloaded = reload(mapping);
    match result {
        Err(error) => Err(error),
        Ok(()) => reloaded,
    }
}

fn entry_index(mapping: &Mapping, id: &str) -> Result<usize, AppError> {
    document_number(id)?;
    mapping
        .data
        .entries
        .iter()
        .position(|e| e.doc_id == id)
        .ok_or_else(|| AppError::new("unknown_document"))
}
fn check_source(pair: &SyncPair, entry: &MappingEntry, expected: &str) -> Result<(), AppError> {
    let path = pair.source_folder.join(&entry.relative_path);
    let witness = ValidatedWrite::new(&pair.source_folder, &path)?;
    if hash_output(&pair.source_folder, &path)?.as_deref() != Some(expected) {
        return Err(AppError::new("source_changed"));
    }
    witness.validate()
}
fn timestamp(value: &str) -> Result<OffsetDateTime, AppError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| AppError::new("invalid_review"))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn review_bytes(review: &ReviewRecord) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec_pretty(review).map_err(|_| AppError::new("invalid_review"))
}
fn review_path(source: &Path, id: &str) -> PathBuf {
    source.join("_redactio/reviews").join(format!("{id}.json"))
}
fn temporary_name() -> String {
    format!(".redactio-commit-{}.tmp", Uuid::new_v4())
}
fn read_bytes(root: &Path, path: &Path) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    open_validated_read(root, path)?
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(AppError::new("record_too_large"));
    }
    Ok(bytes)
}
fn stage(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let write = ValidatedWrite::new(root, path)?;
    if let Some(hash) = hash_output(root, path)? {
        if hash != digest(bytes) {
            return Err(AppError::new("output_conflict"));
        }
        return write.validate();
    }
    write.create_atomic(bytes)
}
fn staged_bytes(root: &Path, path: &Path, expected: &str) -> Result<Option<Vec<u8>>, AppError> {
    match read_bytes(root, path) {
        Ok(bytes) if digest(&bytes) == expected => Ok(Some(bytes)),
        Ok(_) => Err(AppError::new("output_conflict")),
        Err(e) if e.code == "path_unavailable" => Ok(None),
        Err(e) => Err(e),
    }
}
fn ensure_reviews(source: &Path) -> Result<(), AppError> {
    for relative in ["_redactio", "_redactio/reviews"] {
        let path = source.join(relative);
        match super::super::paths::reject_links(&path) {
            Ok(()) => (),
            Err(e) if e.code == "path_unavailable" => {
                ValidatedWrite::new(source, &path)?.create_directory()?
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn cleanup(pair: &SyncPair, pending: &PendingCommit) -> Result<(), AppError> {
    for (root, path, expected) in [
        (
            &pair.target_folder,
            pair.target_folder.join(&pending.output_temporary),
            &pending.generation.output_hash,
        ),
        (
            &pair.source_folder,
            pair.source_folder
                .join("_redactio/reviews")
                .join(&pending.review_temporary),
            &pending.generation.review_hash,
        ),
    ] {
        let write = ValidatedWrite::new(root, &path)?;
        if let Some(hash) = hash_output(root, &path)? {
            if hash != *expected {
                return Err(AppError::new("output_conflict"));
            }
            write.remove()?;
        }
    }
    Ok(())
}
