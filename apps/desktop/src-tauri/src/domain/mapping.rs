use super::{
    paths::{canonical_directory, validate_roots},
    storage::{open_validated_read, ValidatedWrite},
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

pub const MAPPING_FILE: &str = "_document-mapping.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub source_hash: String,
    pub revision: String,
    pub output_hash: String,
    pub review_hash: String,
    pub first_processed_at: String,
    pub last_processed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentState {
    New,
    Current,
    Stale,
    MissingOutput,
    Conflict,
    MissingSource,
    RecoveryPending,
}

pub fn classify(
    observed_source: Option<&str>,
    observed_output: Option<&str>,
    current_revision: &str,
    committed: Option<&Generation>,
    pending: bool,
) -> DocumentState {
    if pending
        || committed.is_some_and(|generation| {
            !valid_hash(&generation.source_hash)
                || !valid_hash(&generation.output_hash)
                || !valid_hash(&generation.review_hash)
                || generation.revision.is_empty()
                || OffsetDateTime::parse(&generation.first_processed_at, &Rfc3339).is_err()
                || OffsetDateTime::parse(&generation.last_processed_at, &Rfc3339).is_err()
        })
    {
        return DocumentState::RecoveryPending;
    }
    let Some(source) = observed_source else {
        return DocumentState::MissingSource;
    };
    let Some(generation) = committed else {
        return if observed_output.is_some() {
            DocumentState::Conflict
        } else {
            DocumentState::New
        };
    };
    let Some(output) = observed_output else {
        return DocumentState::MissingOutput;
    };
    if output != generation.output_hash {
        return DocumentState::Conflict;
    }
    if source != generation.source_hash || current_revision != generation.revision {
        return DocumentState::Stale;
    }
    DocumentState::Current
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingEntry {
    pub doc_id: String,
    pub relative_path: String,
    pub reserved_at: String,
    pub committed: Option<Generation>,
}

/// Shared persisted schema: registry re-add and document operations use one parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingData {
    pub schema_version: u8,
    pub sync_pair_id: Uuid,
    pub target_folder: PathBuf,
    pub next_document_number: u64,
    pub entries: Vec<MappingEntry>,
}

impl MappingData {
    pub fn empty(sync_pair_id: Uuid, target_folder: PathBuf) -> Self {
        Self {
            schema_version: 1,
            sync_pair_id,
            target_folder,
            next_document_number: 1,
            entries: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1
            || self.sync_pair_id.is_nil()
            || self.next_document_number == 0
            || !self.target_folder.is_absolute()
        {
            return Err(AppError::new("invalid_mapping"));
        }
        let mut numbers = HashSet::new();
        let mut paths = HashSet::new();
        for entry in &self.entries {
            let number = document_number(&entry.doc_id)?;
            if number >= self.next_document_number
                || !numbers.insert(number)
                || !paths.insert(&entry.relative_path)
                || validate_relative_path(&entry.relative_path).is_err()
                || OffsetDateTime::parse(&entry.reserved_at, &Rfc3339).is_err()
            {
                return Err(AppError::new("invalid_mapping"));
            }
            if let Some(generation) = &entry.committed {
                generation.validate()?;
            }
        }
        Ok(())
    }

    pub fn read(source: &Path) -> Result<Self, AppError> {
        let mut bytes = Vec::new();
        open_validated_read(source, &source.join(MAPPING_FILE))
            .map_err(|error| {
                if error.code == "path_unavailable" {
                    AppError::new("mapping_missing")
                } else {
                    error
                }
            })?
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(AppError::new("invalid_mapping"));
        }
        let data: Self =
            serde_json::from_slice(&bytes).map_err(|_| AppError::new("invalid_mapping"))?;
        data.validate()?;
        Ok(data)
    }

    pub fn validate_binding(&self, pair_id: Uuid, target: &Path) -> Result<(), AppError> {
        if self.sync_pair_id != pair_id {
            return Err(AppError::new("mapping_pair_mismatch"));
        }
        if !super::paths::same_directory(&self.target_folder, target)? {
            return Err(AppError::new("mapping_target_mismatch"));
        }
        Ok(())
    }
}

impl Generation {
    fn validate(&self) -> Result<(), AppError> {
        let first = OffsetDateTime::parse(&self.first_processed_at, &Rfc3339)
            .map_err(|_| AppError::new("invalid_mapping"))?;
        let last = OffsetDateTime::parse(&self.last_processed_at, &Rfc3339)
            .map_err(|_| AppError::new("invalid_mapping"))?;
        if !valid_hash(&self.source_hash)
            || !valid_hash(&self.output_hash)
            || !valid_hash(&self.review_hash)
            || Uuid::parse_str(&self.revision).map_or(true, |id| id.is_nil())
            || last < first
        {
            return Err(AppError::new("invalid_mapping"));
        }
        Ok(())
    }
}

pub fn document_number(id: &str) -> Result<u64, AppError> {
    let digits = id
        .strip_prefix("doc-")
        .ok_or_else(|| AppError::new("invalid_mapping"))?;
    let number: u64 = digits
        .parse()
        .map_err(|_| AppError::new("invalid_mapping"))?;
    if digits.len() < 4
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || number == 0
        || id != format!("doc-{number:04}")
    {
        return Err(AppError::new("invalid_mapping"));
    }
    Ok(number)
}

pub fn validate_relative_path(path: &str) -> Result<(), AppError> {
    if path.is_empty()
        || path.contains(['\\', ':', '\0'])
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(AppError::new("invalid_path"));
    }
    Ok(())
}

pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug)]
pub struct Mapping {
    source: PathBuf,
    target: PathBuf,
    pair_id: Uuid,
    data: MappingData,
}

impl Mapping {
    pub fn load(source: &Path, pair_id: Uuid, target: &Path) -> Result<Self, AppError> {
        super::recovery::ensure_recovered(source, pair_id)?;
        let source = canonical_directory(source)?;
        let target = canonical_directory(target)?;
        let data = MappingData::read(&source)?;
        data.validate_binding(pair_id, &target)?;
        Ok(Self {
            source,
            target,
            pair_id,
            data,
        })
    }

    pub fn entries(&self) -> &[MappingEntry] {
        &self.data.entries
    }

    /// Acquires a short reservation lock. Validate other pair/config roots before calling.
    /// Every attempt reloads disk truth, including retries after indeterminate atomic-write errors.
    pub fn reserve(&mut self, relative_path: &str) -> Result<String, AppError> {
        let guard = CollectionGuard::acquire(&self.source)?;
        self.reserve_with_guard(relative_path, &guard)
    }

    /// For a larger operation that already retains its collection guard (no nested lock).
    pub fn reserve_with_guard(
        &mut self,
        relative_path: &str,
        guard: &CollectionGuard,
    ) -> Result<String, AppError> {
        guard.validate(&self.source)?;
        super::recovery::ensure_recovered(&self.source, self.pair_id)?;
        validate_relative_path(relative_path)?;
        validate_roots(&self.source, &self.target, &[])?;
        let write = ValidatedWrite::new(&self.source, &self.source.join(MAPPING_FILE))?;
        let mut data = MappingData::read(&self.source)?;
        data.validate_binding(self.pair_id, &self.target)?;
        if let Some(entry) = data
            .entries
            .iter()
            .find(|entry| entry.relative_path == relative_path)
        {
            write.validate()?;
            let id = entry.doc_id.clone();
            self.data = data;
            return Ok(id);
        }
        let number = data.next_document_number;
        data.next_document_number = number
            .checked_add(1)
            .ok_or_else(|| AppError::new("document_ids_exhausted"))?;
        let doc_id = format!("doc-{number:04}");
        data.entries.push(MappingEntry {
            doc_id: doc_id.clone(),
            relative_path: relative_path.into(),
            reserved_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| AppError::new("invalid_mapping"))?,
            committed: None,
        });
        let bytes =
            serde_json::to_vec_pretty(&data).map_err(|_| AppError::new("invalid_mapping"))?;
        write.write_atomic(&bytes)?;
        self.data = data;
        Ok(doc_id)
    }
}

/// OS lock, retained across load/reserve/commit by processing and recovery callers.
/// Never remove the lock file: unlinking a held lock would allow a second writer.
#[derive(Debug)]
pub struct CollectionGuard {
    source: PathBuf,
    _file: File,
    witness: ValidatedWrite,
}

impl CollectionGuard {
    pub fn acquire(source: &Path) -> Result<Self, AppError> {
        let source = canonical_directory(source)?;
        let path = source.join(".redactio-lock");
        match open_validated_read(&source, &path) {
            Ok(_) => (),
            Err(error) if error.code == "path_unavailable" => {
                ValidatedWrite::new(&source, &path)?.write_atomic(b"")?;
            }
            Err(error) => return Err(error),
        }
        let witness = ValidatedWrite::new(&source, &path)?;
        let file = open_validated_read(&source, &path)?;
        file.try_lock().map_err(|_| AppError {
            code: "file_busy".into(),
            retryable: true,
        })?;
        witness.validate()?;
        Ok(Self {
            source,
            _file: file,
            witness,
        })
    }

    pub fn validate(&self, source: &Path) -> Result<(), AppError> {
        if !super::paths::same_directory(&self.source, source)? {
            return Err(AppError::new("mapping_pair_mismatch"));
        }
        self.witness.validate()
    }
}
