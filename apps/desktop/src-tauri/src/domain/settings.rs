use super::{
    paths::{canonical_directory, validate_roots},
    storage::ValidatedWrite,
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

const MAPPING_FILE: &str = "_document-mapping.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityType {
    Person,
    Location,
    EmailAddress,
    PhoneNumber,
    IbanCode,
    IpAddress,
    Url,
    DateTime,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum CustomRule {
    Regex {
        id: Uuid,
        entity_type: EntityType,
        enabled: bool,
        pattern: String,
    },
    Words {
        id: Uuid,
        entity_type: EntityType,
        enabled: bool,
        words: Vec<String>,
    },
}

impl CustomRule {
    fn id(&self) -> Uuid {
        match self {
            Self::Regex { id, .. } | Self::Words { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingConfig {
    pub model: String,
    pub enabled_entities: Vec<EntityType>,
    pub custom_rules: Vec<CustomRule>,
    pub include_positions: bool,
}

impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            model: "de_core_news_lg".into(),
            enabled_entities: vec![
                EntityType::Person,
                EntityType::Location,
                EntityType::EmailAddress,
                EntityType::PhoneNumber,
                EntityType::IbanCode,
                EntityType::IpAddress,
                EntityType::Url,
                EntityType::DateTime,
            ],
            custom_rules: Vec::new(),
            include_positions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingFingerprint {
    pub engine_version: String,
    pub extraction_version: String,
    pub model_name: String,
    pub model_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncPair {
    pub id: Uuid,
    pub name: String,
    pub source_folder: PathBuf,
    pub target_folder: PathBuf,
    pub created_at: String,
    pub processing_revision: Uuid,
    pub config: ProcessingConfig,
    pub processing_fingerprint: Option<ProcessingFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u8,
    pub sync_pairs: Vec<SyncPair>,
    pub selected_sync_pair_id: Option<Uuid>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            sync_pairs: Vec::new(),
            selected_sync_pair_id: None,
        }
    }
}

impl Settings {
    pub fn add(&mut self, name: &str, source: &Path, target: &Path) -> Result<Uuid, AppError> {
        let name = checked_name(
            name,
            self.sync_pairs.iter().map(|pair| (&pair.name, pair.id)),
            None,
        )?;
        let other_roots = self.roots();
        let (source, target) = validate_roots(source, target, &other_roots)?;
        let mapping_path = source.join(MAPPING_FILE);
        let mapping_write = ValidatedWrite::new(&source, &mapping_path)?;
        let mapping = match fs::read(&mapping_path) {
            Ok(bytes) => {
                let mapping: Mapping =
                    serde_json::from_slice(&bytes).map_err(|_| AppError::new("invalid_mapping"))?;
                mapping.validate()?;
                if canonical_directory(&mapping.target_folder)? != target {
                    return Err(AppError::new("mapping_target_mismatch"));
                }
                mapping_write.validate()?;
                mapping
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::read_dir(&target)?.next().is_some() {
                    return Err(AppError::new("target_not_empty"));
                }
                let mapping = Mapping {
                    schema_version: 1,
                    sync_pair_id: Uuid::new_v4(),
                    target_folder: target.clone(),
                    next_document_number: 1,
                    entries: Vec::new(),
                };
                let bytes = serde_json::to_vec_pretty(&mapping)
                    .map_err(|_| AppError::new("invalid_mapping"))?;
                mapping_write.write_atomic(&bytes)?;
                mapping
            }
            Err(error) => return Err(error.into()),
        };
        if self
            .sync_pairs
            .iter()
            .any(|pair| pair.id == mapping.sync_pair_id)
        {
            return Err(AppError::new("duplicate_pair_id"));
        }
        let pair = SyncPair {
            id: mapping.sync_pair_id,
            name,
            source_folder: source,
            target_folder: target,
            created_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| AppError::new("invalid_settings"))?,
            processing_revision: Uuid::new_v4(),
            config: ProcessingConfig::default(),
            processing_fingerprint: None,
        };
        let id = pair.id;
        self.sync_pairs.push(pair);
        if self.selected_sync_pair_id.is_none() {
            self.selected_sync_pair_id = Some(id);
        }
        Ok(id)
    }

    pub fn rename(&mut self, id: Uuid, name: &str) -> Result<(), AppError> {
        let name = checked_name(
            name,
            self.sync_pairs.iter().map(|pair| (&pair.name, pair.id)),
            Some(id),
        )?;
        let pair = self
            .sync_pairs
            .iter_mut()
            .find(|pair| pair.id == id)
            .ok_or_else(|| AppError::new("unknown_pair"))?;
        pair.name = name;
        Ok(())
    }

    pub fn select(&mut self, id: Uuid) -> Result<(), AppError> {
        if !self.sync_pairs.iter().any(|pair| pair.id == id) {
            return Err(AppError::new("unknown_pair"));
        }
        self.selected_sync_pair_id = Some(id);
        Ok(())
    }

    pub fn remove(&mut self, id: Uuid) -> Result<(), AppError> {
        let index = self
            .sync_pairs
            .iter()
            .position(|pair| pair.id == id)
            .ok_or_else(|| AppError::new("unknown_pair"))?;
        self.sync_pairs.remove(index);
        if self.selected_sync_pair_id == Some(id) {
            self.selected_sync_pair_id = self
                .sync_pairs
                .get(index)
                .or_else(|| {
                    index
                        .checked_sub(1)
                        .and_then(|previous| self.sync_pairs.get(previous))
                })
                .map(|pair| pair.id);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1 {
            return Err(AppError::new("unsupported_settings_schema"));
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for pair in &self.sync_pairs {
            if pair.id.is_nil() || pair.processing_revision.is_nil() || !ids.insert(pair.id) {
                return Err(AppError::new("invalid_settings"));
            }
            let trimmed = pair.name.trim();
            if trimmed.is_empty() || trimmed != pair.name || !names.insert(fold_name(trimmed)) {
                return Err(AppError::new("invalid_settings"));
            }
            if !pair.source_folder.is_absolute()
                || !pair.target_folder.is_absolute()
                || OffsetDateTime::parse(&pair.created_at, &Rfc3339).is_err()
            {
                return Err(AppError::new("invalid_settings"));
            }
            if pair.config.model.trim().is_empty() {
                return Err(AppError::new("invalid_settings"));
            }
            let mut entities = HashSet::new();
            if pair
                .config
                .enabled_entities
                .iter()
                .any(|entity| *entity == EntityType::Custom || !entities.insert(*entity))
            {
                return Err(AppError::new("invalid_settings"));
            }
            let mut rule_ids = HashSet::new();
            for rule in &pair.config.custom_rules {
                if rule.id().is_nil() || !rule_ids.insert(rule.id()) {
                    return Err(AppError::new("invalid_settings"));
                }
            }
        }
        if self
            .selected_sync_pair_id
            .is_some_and(|id| !ids.contains(&id))
        {
            return Err(AppError::new("invalid_settings"));
        }
        Ok(())
    }

    pub fn validate_roots_with(&self, extra_roots: &[PathBuf]) -> Result<(), AppError> {
        self.validate()?;
        for (index, pair) in self.sync_pairs.iter().enumerate() {
            let mut others = extra_roots.to_vec();
            for (other_index, other) in self.sync_pairs.iter().enumerate() {
                if index != other_index {
                    others.extend([other.source_folder.clone(), other.target_folder.clone()]);
                }
            }
            validate_roots(&pair.source_folder, &pair.target_folder, &others)?;
        }
        Ok(())
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.sync_pairs
            .iter()
            .flat_map(|pair| [pair.source_folder.clone(), pair.target_folder.clone()])
            .collect()
    }
}

pub fn load_settings(path: &Path) -> Result<Settings, AppError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(error) => return Err(error.into()),
    };
    let settings: Settings =
        serde_json::from_slice(&bytes).map_err(|_| AppError::new("invalid_settings"))?;
    settings.validate()?;
    Ok(settings)
}

pub fn save_settings(path: &Path, settings: &Settings) -> Result<(), AppError> {
    settings.validate()?;
    let parent = path.parent().ok_or_else(|| AppError::new("invalid_path"))?;
    let write = ValidatedWrite::new(parent, path)?;
    match fs::read(path) {
        Ok(bytes) => {
            let current: Settings =
                serde_json::from_slice(&bytes).map_err(|_| AppError::new("invalid_settings"))?;
            current.validate()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let bytes =
        serde_json::to_vec_pretty(settings).map_err(|_| AppError::new("invalid_settings"))?;
    write.write_atomic(&bytes)
}

fn checked_name<'a>(
    name: &str,
    mut existing: impl Iterator<Item = (&'a String, Uuid)>,
    except: Option<Uuid>,
) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::new("invalid_pair_name"));
    }
    let folded = fold_name(name);
    if existing.any(|(other, id)| Some(id) != except && fold_name(other) == folded) {
        return Err(AppError::new("duplicate_pair_name"));
    }
    Ok(name.into())
}

fn fold_name(name: &str) -> String {
    name.chars().flat_map(char::to_lowercase).collect()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    schema_version: u8,
    sync_pair_id: Uuid,
    target_folder: PathBuf,
    next_document_number: u64,
    entries: Vec<serde_json::Value>,
}

impl Mapping {
    fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1
            || self.sync_pair_id.is_nil()
            || self.next_document_number == 0
            || !self.target_folder.is_absolute()
        {
            return Err(AppError::new("invalid_mapping"));
        }
        Ok(())
    }
}
