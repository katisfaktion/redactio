use crate::{domain::settings::EntityType, error::AppError, protocol::ModelInfo};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Component, Path},
};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const CATALOG: &str = include_str!("../../../sidecar/src/redactio_sidecar/model_catalog.json");

struct Nullable<T>(Option<T>);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Nullable<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(Option::<T>::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HashAlgorithm {
    GitSha1,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamHash {
    pub algorithm: HashAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Artifact {
    pub filename: String,
    pub size: u64,
    pub upstream_hash: UpstreamHash,
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactWire {
    filename: String,
    size: u64,
    upstream_hash: UpstreamHash,
    sha256: Nullable<String>,
}

impl TryFrom<ArtifactWire> for Artifact {
    type Error = String;
    fn try_from(value: ArtifactWire) -> Result<Self, Self::Error> {
        if !filename(&value.filename)
            || value.size > MAX_SAFE_INTEGER
            || !hash(
                &value.upstream_hash.value,
                matches!(value.upstream_hash.algorithm, HashAlgorithm::GitSha1),
            )
            || value
                .sha256
                .0
                .as_deref()
                .is_some_and(|value| !hash(value, false))
        {
            return Err("invalid artifact".into());
        }
        Ok(Self {
            filename: value.filename,
            size: value.size,
            upstream_hash: value.upstream_hash,
            sha256: value.sha256.0,
        })
    }
}
impl<'de> Deserialize<'de> for Artifact {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["filename", "size", "upstream_hash", "sha256"]) {
            return Err(D::Error::custom("missing artifact field"));
        }
        serde_json::from_value::<ArtifactWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelType {
    Bert,
    DebertaV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Architecture {
    BertForTokenClassification,
    DebertaV2ForTokenClassification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelDescriptor {
    pub name: String,
    pub version: String,
    pub repository: String,
    pub title: String,
    pub license: Option<String>,
    pub model_type: ModelType,
    pub architecture: Architecture,
    pub entity_types: Vec<String>,
    pub window_tokens: u64,
    pub stride_tokens: u64,
    pub special_tokens: Option<u64>,
    pub files: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelDescriptorWire {
    name: String,
    version: String,
    repository: String,
    title: String,
    license: Nullable<String>,
    model_type: ModelType,
    architecture: Architecture,
    entity_types: Vec<String>,
    window_tokens: u64,
    stride_tokens: u64,
    special_tokens: Nullable<u64>,
    files: Vec<Artifact>,
}

impl TryFrom<ModelDescriptorWire> for ModelDescriptor {
    type Error = String;
    fn try_from(value: ModelDescriptorWire) -> Result<Self, Self::Error> {
        let matching_architecture = matches!(
            (&value.model_type, &value.architecture),
            (ModelType::Bert, Architecture::BertForTokenClassification)
                | (
                    ModelType::DebertaV2,
                    Architecture::DebertaV2ForTokenClassification
                )
        );
        let content = value
            .window_tokens
            .checked_sub(value.special_tokens.0.unwrap_or(0))
            .ok_or("invalid window")?;
        let expected_stride = std::cmp::min(
            std::cmp::max(1, value.window_tokens / 4),
            content.saturating_sub(1),
        );
        let mut sorted = value.entity_types.clone();
        sorted.sort_unstable();
        sorted.dedup();
        let filenames = value
            .files
            .iter()
            .map(|file| &file.filename)
            .collect::<HashSet<_>>();
        if !opaque(&value.name)
            || !hash(&value.version, true)
            || !repository(&value.repository)
            || !matching_architecture
            || value.window_tokens < 2
            || value.window_tokens > MAX_SAFE_INTEGER
            || value.stride_tokens > MAX_SAFE_INTEGER
            || value
                .special_tokens
                .0
                .is_some_and(|tokens| tokens > MAX_SAFE_INTEGER)
            || content < 2
            || value.stride_tokens != expected_stride
            || value.entity_types.is_empty()
            || sorted != value.entity_types
            || value
                .entity_types
                .iter()
                .any(|label| !entity(label) || generic_label(label))
            || filenames.len() != value.files.len()
            || (value.name.starts_with("hf:")
                && value.name != format!("hf:{}@{}", value.repository, value.version))
        {
            return Err("invalid model descriptor".into());
        }
        Ok(Self {
            name: value.name,
            version: value.version,
            repository: value.repository,
            title: value.title,
            license: value.license.0,
            model_type: value.model_type,
            architecture: value.architecture,
            entity_types: value.entity_types,
            window_tokens: value.window_tokens,
            stride_tokens: value.stride_tokens,
            special_tokens: value.special_tokens.0,
            files: value.files,
        })
    }
}
impl<'de> Deserialize<'de> for ModelDescriptor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(
            &value,
            &[
                "name",
                "version",
                "repository",
                "title",
                "license",
                "model_type",
                "architecture",
                "entity_types",
                "window_tokens",
                "stride_tokens",
                "special_tokens",
                "files",
            ],
        ) {
            return Err(D::Error::custom("missing model descriptor field"));
        }
        serde_json::from_value::<ModelDescriptorWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogKey {
    Biomedbert,
    Hugginglil,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    pub key: CatalogKey,
    pub directory: String,
    pub descriptor: ModelDescriptor,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntryWire {
    key: CatalogKey,
    directory: String,
    descriptor: ModelDescriptor,
}
impl TryFrom<CatalogEntryWire> for CatalogEntry {
    type Error = String;
    fn try_from(value: CatalogEntryWire) -> Result<Self, Self::Error> {
        if !directory(&value.directory) {
            return Err("invalid catalog entry".into());
        }
        Ok(Self {
            key: value.key,
            directory: value.directory,
            descriptor: value.descriptor,
        })
    }
}
impl<'de> Deserialize<'de> for CatalogEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["key", "directory", "descriptor"]) {
            return Err(D::Error::custom("missing catalog entry field"));
        }
        serde_json::from_value::<CatalogEntryWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Ready,
    Available,
    Removing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRecord {
    pub descriptor: ModelDescriptor,
    pub path: Option<String>,
    pub state: ModelState,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelRecordWire {
    descriptor: ModelDescriptor,
    path: Nullable<String>,
    state: ModelState,
}
impl TryFrom<ModelRecordWire> for ModelRecord {
    type Error = String;
    fn try_from(value: ModelRecordWire) -> Result<Self, Self::Error> {
        if value.path.0.as_deref().is_some_and(|path| !directory(path))
            || (matches!(value.state, ModelState::Ready)
                && (value.path.0.is_none()
                    || value.descriptor.special_tokens.is_none()
                    || value
                        .descriptor
                        .files
                        .iter()
                        .any(|file| file.sha256.is_none())))
        {
            return Err("invalid model record".into());
        }
        Ok(Self {
            descriptor: value.descriptor,
            path: value.path.0,
            state: value.state,
        })
    }
}
impl<'de> Deserialize<'de> for ModelRecord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["descriptor", "path", "state"]) {
            return Err(D::Error::custom("missing model record field"));
        }
        serde_json::from_value::<ModelRecordWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyEntry {
    pub name: String,
    pub version: String,
    pub path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyEntryWire {
    name: String,
    version: String,
    path: String,
}
impl TryFrom<LegacyEntryWire> for LegacyEntry {
    type Error = String;
    fn try_from(value: LegacyEntryWire) -> Result<Self, Self::Error> {
        if !opaque(&value.name) || !legacy_path(&value.path) {
            return Err("invalid legacy entry".into());
        }
        Ok(Self {
            name: value.name,
            version: value.version,
            path: value.path,
        })
    }
}
impl<'de> Deserialize<'de> for LegacyEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["name", "version", "path"]) {
            return Err(D::Error::custom("missing legacy entry field"));
        }
        serde_json::from_value::<LegacyEntryWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRegistry {
    pub schema_version: u8,
    pub models: Vec<ModelRecord>,
    pub legacy_unavailable: Vec<LegacyEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelRegistryWire {
    schema_version: u8,
    models: Vec<ModelRecord>,
    legacy_unavailable: Vec<LegacyEntry>,
}
impl TryFrom<ModelRegistryWire> for ModelRegistry {
    type Error = String;
    fn try_from(value: ModelRegistryWire) -> Result<Self, Self::Error> {
        let record_names = value
            .models
            .iter()
            .map(|record| &record.descriptor.name)
            .collect::<HashSet<_>>();
        let names = value
            .models
            .iter()
            .map(|record| &record.descriptor.name)
            .chain(value.legacy_unavailable.iter().map(|entry| &entry.name))
            .collect::<HashSet<_>>();
        let paths = value
            .models
            .iter()
            .filter_map(|record| record.path.as_ref())
            .collect::<HashSet<_>>();
        if value.schema_version != 2
            || record_names.len() != value.models.len()
            || names.len() != value.models.len() + value.legacy_unavailable.len()
            || paths.len()
                != value
                    .models
                    .iter()
                    .filter(|record| record.path.is_some())
                    .count()
        {
            return Err("unsupported registry".into());
        }
        Ok(Self {
            schema_version: value.schema_version,
            models: value.models,
            legacy_unavailable: value.legacy_unavailable,
        })
    }
}
impl<'de> Deserialize<'de> for ModelRegistry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["schema_version", "models", "legacy_unavailable"]) {
            return Err(D::Error::custom("missing registry field"));
        }
        serde_json::from_value::<ModelRegistryWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelCatalog {
    pub schema_version: u8,
    pub models: Vec<CatalogEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCatalogWire {
    schema_version: u8,
    models: Vec<CatalogEntry>,
}
impl TryFrom<ModelCatalogWire> for ModelCatalog {
    type Error = String;
    fn try_from(value: ModelCatalogWire) -> Result<Self, Self::Error> {
        if value.schema_version != 1
            || value
                .models
                .iter()
                .map(|entry| &entry.key)
                .collect::<HashSet<_>>()
                .len()
                != value.models.len()
        {
            return Err("invalid catalog".into());
        }
        Ok(Self {
            schema_version: value.schema_version,
            models: value.models,
        })
    }
}
impl<'de> Deserialize<'de> for ModelCatalog {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["schema_version", "models"]) {
            return Err(D::Error::custom("missing catalog field"));
        }
        serde_json::from_value::<ModelCatalogWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ManagedState {
    Available,
    Ready,
    Invalid,
    Removing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsedByPair {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsedByPairWire {
    id: String,
    name: String,
}
impl TryFrom<UsedByPairWire> for UsedByPair {
    type Error = String;
    fn try_from(value: UsedByPairWire) -> Result<Self, Self::Error> {
        if !uuid(&value.id) {
            return Err("invalid pair".into());
        }
        Ok(Self {
            id: value.id,
            name: value.name,
        })
    }
}
impl<'de> Deserialize<'de> for UsedByPair {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["id", "name"]) {
            return Err(D::Error::custom("missing pair field"));
        }
        serde_json::from_value::<UsedByPairWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManagedModel {
    pub name: String,
    pub version: String,
    pub repository: String,
    pub title: String,
    pub license: Option<String>,
    pub entity_types: Vec<String>,
    pub window_tokens: Option<u64>,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub state: ManagedState,
    pub catalog_key: Option<CatalogKey>,
    pub used_by_pairs: Vec<UsedByPair>,
    pub error: Option<AppError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedModelWire {
    name: String,
    version: String,
    repository: String,
    title: String,
    license: Nullable<String>,
    entity_types: Vec<String>,
    window_tokens: Nullable<u64>,
    download_bytes: u64,
    installed_bytes: u64,
    state: ManagedState,
    catalog_key: Nullable<CatalogKey>,
    used_by_pairs: Vec<UsedByPair>,
    error: Nullable<AppError>,
}
impl TryFrom<ManagedModelWire> for ManagedModel {
    type Error = String;
    fn try_from(value: ManagedModelWire) -> Result<Self, Self::Error> {
        let mut sorted = value.entity_types.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if !opaque(&value.name)
            || !hash(&value.version, true)
            || !repository(&value.repository)
            || value.entity_types.is_empty()
            || sorted != value.entity_types
            || value
                .entity_types
                .iter()
                .any(|label| !entity(label) || generic_label(label))
            || value
                .window_tokens
                .0
                .is_some_and(|tokens| !(2..=MAX_SAFE_INTEGER).contains(&tokens))
            || value.download_bytes > MAX_SAFE_INTEGER
            || value.installed_bytes > MAX_SAFE_INTEGER
            || (value.name.starts_with("hf:")
                && value.name != format!("hf:{}@{}", value.repository, value.version))
        {
            return Err("invalid managed model".into());
        }
        Ok(Self {
            name: value.name,
            version: value.version,
            repository: value.repository,
            title: value.title,
            license: value.license.0,
            entity_types: value.entity_types,
            window_tokens: value.window_tokens.0,
            download_bytes: value.download_bytes,
            installed_bytes: value.installed_bytes,
            state: value.state,
            catalog_key: value.catalog_key.0,
            used_by_pairs: value.used_by_pairs,
            error: value.error.0,
        })
    }
}
impl<'de> Deserialize<'de> for ManagedModel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(
            &value,
            &[
                "name",
                "version",
                "repository",
                "title",
                "license",
                "entity_types",
                "window_tokens",
                "download_bytes",
                "installed_bytes",
                "state",
                "catalog_key",
                "used_by_pairs",
                "error",
            ],
        ) {
            return Err(D::Error::custom("missing managed model field"));
        }
        serde_json::from_value::<ManagedModelWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ModelSource {
    Catalog { key: CatalogKey },
    Url { url: String },
    Receipt { name: String },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum ModelSourceWire {
    Catalog { key: CatalogKey },
    Url { url: String },
    Receipt { name: String },
}
impl TryFrom<ModelSourceWire> for ModelSource {
    type Error = String;
    fn try_from(value: ModelSourceWire) -> Result<Self, Self::Error> {
        match value {
            ModelSourceWire::Catalog { key } => Ok(Self::Catalog { key }),
            ModelSourceWire::Url { url } if huggingface_url(&url) => Ok(Self::Url { url }),
            ModelSourceWire::Receipt { name } if opaque(&name) => Ok(Self::Receipt { name }),
            _ => Err("invalid model source".into()),
        }
    }
}
impl<'de> Deserialize<'de> for ModelSource {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let fields = match value.get("kind").and_then(serde_json::Value::as_str) {
            Some("catalog") => &["kind", "key"][..],
            Some("url") => &["kind", "url"],
            Some("receipt") => &["kind", "name"],
            _ => return Err(D::Error::custom("invalid model source")),
        };
        if !required_fields(&value, fields) {
            return Err(D::Error::custom("missing model source field"));
        }
        serde_json::from_value::<ModelSourceWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckedModel {
    pub plan_id: String,
    pub model: ManagedModel,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckedModelWire {
    plan_id: String,
    model: ManagedModel,
}
impl TryFrom<CheckedModelWire> for CheckedModel {
    type Error = String;
    fn try_from(value: CheckedModelWire) -> Result<Self, Self::Error> {
        if !uuid(&value.plan_id) {
            return Err("invalid checked model".into());
        }
        Ok(Self {
            plan_id: value.plan_id,
            model: value.model,
        })
    }
}
impl<'de> Deserialize<'de> for CheckedModel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(&value, &["plan_id", "model"]) {
            return Err(D::Error::custom("missing checked model field"));
        }
        serde_json::from_value::<CheckedModelWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelJobStage {
    Downloading,
    Validating,
    Ready,
    Cancelled,
    Failed,
    Removing,
    Removed,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelJob {
    pub job_id: String,
    pub model_name: String,
    pub stage: ModelJobStage,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<AppError>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelJobWire {
    job_id: String,
    model_name: String,
    stage: ModelJobStage,
    downloaded_bytes: u64,
    total_bytes: u64,
    error: Nullable<AppError>,
}
impl TryFrom<ModelJobWire> for ModelJob {
    type Error = String;
    fn try_from(value: ModelJobWire) -> Result<Self, Self::Error> {
        if !uuid(&value.job_id)
            || !opaque(&value.model_name)
            || value.downloaded_bytes > value.total_bytes
            || value.total_bytes > MAX_SAFE_INTEGER
        {
            return Err("invalid model job".into());
        }
        Ok(Self {
            job_id: value.job_id,
            model_name: value.model_name,
            stage: value.stage,
            downloaded_bytes: value.downloaded_bytes,
            total_bytes: value.total_bytes,
            error: value.error.0,
        })
    }
}
impl<'de> Deserialize<'de> for ModelJob {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        if !required_fields(
            &value,
            &[
                "job_id",
                "model_name",
                "stage",
                "downloaded_bytes",
                "total_bytes",
                "error",
            ],
        ) {
            return Err(D::Error::custom("missing model job field"));
        }
        serde_json::from_value::<ModelJobWire>(value)
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyManifest {
    models: Vec<LegacyEntry>,
}

pub fn selection_name(repository: &str, revision: &str) -> String {
    if let Ok(catalog) = catalog_models() {
        if let Some(entry) = catalog.iter().find(|entry| {
            entry.descriptor.repository == repository && entry.descriptor.version == revision
        }) {
            return entry.descriptor.name.clone();
        }
    }
    format!("hf:{repository}@{revision}")
}

pub fn catalog_models() -> Result<Vec<CatalogEntry>, AppError> {
    Ok(serde_json::from_str::<ModelCatalog>(CATALOG)
        .map_err(|_| AppError::new("invalid_model_catalog"))?
        .models)
}

pub fn read_registry(root: &Path) -> Result<ModelRegistry, AppError> {
    Ok(read_store(root, &catalog_models()?)?.0)
}

fn read_store(root: &Path, catalog: &[CatalogEntry]) -> Result<(ModelRegistry, bool), AppError> {
    let exists = safe_root(root)?;
    if !exists {
        return Ok((
            ModelRegistry {
                schema_version: 2,
                models: Vec::new(),
                legacy_unavailable: Vec::new(),
            },
            false,
        ));
    }
    let manifest = root.join("manifest.json");
    let bytes = match fs::symlink_metadata(&manifest) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((
                ModelRegistry {
                    schema_version: 2,
                    models: Vec::new(),
                    legacy_unavailable: Vec::new(),
                },
                false,
            ))
        }
        Err(_) => return Err(AppError::new("invalid_model_manifest")),
        Ok(metadata) if crate::domain::paths::is_link(&metadata) || !metadata.is_file() => {
            return Err(AppError::new("invalid_model_manifest"))
        }
        Ok(_) => fs::read(&manifest).map_err(|_| AppError::new("invalid_model_manifest"))?,
    };
    if let Ok(registry) = serde_json::from_slice::<ModelRegistry>(&bytes) {
        return Ok((registry, false));
    }
    let legacy: LegacyManifest =
        serde_json::from_slice(&bytes).map_err(|_| AppError::new("invalid_model_manifest"))?;
    let mut registry = ModelRegistry {
        schema_version: 2,
        models: Vec::new(),
        legacy_unavailable: Vec::new(),
    };
    for legacy in legacy.models {
        if let Some(entry) = catalog.iter().find(|entry| {
            entry.descriptor.name == legacy.name && entry.descriptor.version == legacy.version
        }) {
            let record = ModelRecord {
                descriptor: entry.descriptor.clone(),
                path: Some(legacy.path.clone()),
                state: ModelState::Ready,
            };
            if directory(record.path.as_deref().unwrap()) {
                if let Some(labels) = compatibility(root, &record, true, catalog) {
                    let mut record = record;
                    record.descriptor.entity_types = labels
                        .into_iter()
                        .map(|label| label.as_str().to_owned())
                        .collect();
                    registry.models.push(record);
                    continue;
                }
            }
            registry.legacy_unavailable.push(legacy);
        } else {
            registry.legacy_unavailable.push(legacy);
        }
    }
    let registry = serde_json::from_value(
        serde_json::to_value(registry).map_err(|_| AppError::new("invalid_model_manifest"))?,
    )
    .map_err(|_| AppError::new("invalid_model_manifest"))?;
    Ok((registry, true))
}

pub fn list_models(root: &Path) -> Result<Vec<ModelInfo>, AppError> {
    let catalog = catalog_models()?;
    let (registry, legacy) = read_store(root, &catalog)?;
    Ok(registry
        .models
        .into_iter()
        .filter(|record| matches!(record.state, ModelState::Ready))
        .map(|record| {
            let entity_types = compatibility(root, &record, legacy, &catalog);
            ModelInfo {
                name: record.descriptor.name,
                version: record.descriptor.version,
                compatible: entity_types.is_some(),
                entity_types: entity_types.unwrap_or_default(),
            }
        })
        .collect())
}

pub fn list_managed(root: &Path) -> Result<Vec<ManagedModel>, AppError> {
    let catalog = catalog_models()?;
    let (registry, legacy) = read_store(root, &catalog)?;
    let mut models = catalog
        .iter()
        .map(|entry| {
            managed(
                &entry.descriptor,
                Some(entry.key.clone()),
                ManagedState::Available,
                0,
            )
        })
        .collect::<Vec<_>>();
    for record in registry.models {
        let compatible = compatibility(root, &record, legacy, &catalog);
        let state = match record.state {
            ModelState::Available => ManagedState::Available,
            ModelState::Removing => ManagedState::Removing,
            ModelState::Ready if compatible.is_some() => ManagedState::Ready,
            ModelState::Ready => ManagedState::Invalid,
        };
        let installed = record
            .path
            .as_deref()
            .map(|path| root.join(path))
            .filter(|path| directory_on_disk(path))
            .map(|path| installed_bytes(&path))
            .unwrap_or(0);
        let catalog_key = catalog
            .iter()
            .find(|entry| same_identity(&entry.descriptor, &record.descriptor))
            .map(|entry| entry.key.clone());
        let item = managed(&record.descriptor, catalog_key.clone(), state, installed);
        if let Some(index) = models
            .iter()
            .position(|current| same_identity_fields(current, &record.descriptor))
        {
            models[index] = item;
        } else {
            models.push(item);
        }
    }
    Ok(models)
}

pub(crate) fn managed(
    descriptor: &ModelDescriptor,
    catalog_key: Option<CatalogKey>,
    state: ManagedState,
    installed_bytes: u64,
) -> ManagedModel {
    ManagedModel {
        name: descriptor.name.clone(),
        version: descriptor.version.clone(),
        repository: descriptor.repository.clone(),
        title: descriptor.title.clone(),
        license: descriptor.license.clone(),
        entity_types: descriptor.entity_types.clone(),
        window_tokens: Some(descriptor.window_tokens),
        download_bytes: descriptor.files.iter().map(|file| file.size).sum(),
        installed_bytes,
        state,
        catalog_key,
        used_by_pairs: Vec::new(),
        error: None,
    }
}
fn same_identity(left: &ModelDescriptor, right: &ModelDescriptor) -> bool {
    left.repository == right.repository && left.version == right.version
}
fn same_identity_fields(managed: &ManagedModel, descriptor: &ModelDescriptor) -> bool {
    managed.repository == descriptor.repository && managed.version == descriptor.version
}

fn compatibility(
    root: &Path,
    record: &ModelRecord,
    legacy: bool,
    catalog: &[CatalogEntry],
) -> Option<Vec<EntityType>> {
    let path = record.path.as_deref()?;
    let model = root.join(path);
    if !directory_on_disk(&model)
        || record.descriptor.name != canonical_name(catalog, &record.descriptor)
    {
        return None;
    }
    let required = [
        "model.safetensors",
        "config.json",
        "tokenizer.json",
        "tokenizer_config.json",
    ];
    if required.iter().any(|name| !regular_file(&model.join(name)))
        || !required.iter().all(|name| {
            legacy
                || record
                    .descriptor
                    .files
                    .iter()
                    .any(|file| file.filename == *name)
        })
    {
        return None;
    }
    let receipt_path = model.join("redactio-model.json");
    if !regular_file(&receipt_path)
        || !fs::read(&receipt_path)
            .ok()
            .is_some_and(|receipt| receipt_matches(&receipt, &record.descriptor))
    {
        return None;
    }
    if !legacy
        && (record.descriptor.files.is_empty()
            || record.descriptor.files.iter().any(|file| {
                fs::symlink_metadata(model.join(&file.filename))
                    .ok()
                    .filter(|metadata| {
                        metadata.is_file()
                            && !crate::domain::paths::is_link(metadata)
                            && metadata.len() == file.size
                    })
                    .is_none()
            }))
    {
        return None;
    }
    native_metadata(&model, &record.descriptor, legacy)
}

fn canonical_name(catalog: &[CatalogEntry], descriptor: &ModelDescriptor) -> String {
    catalog
        .iter()
        .find(|entry| {
            entry.descriptor.repository == descriptor.repository
                && entry.descriptor.version == descriptor.version
        })
        .map(|entry| entry.descriptor.name.clone())
        .unwrap_or_else(|| format!("hf:{}@{}", descriptor.repository, descriptor.version))
}

fn native_metadata(
    model: &Path,
    descriptor: &ModelDescriptor,
    legacy: bool,
) -> Option<Vec<EntityType>> {
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(model.join("config.json")).ok()?).ok()?;
    let tokenizer_bytes = fs::read(model.join("tokenizer_config.json")).ok()?;
    let tokenizer: BTreeMap<String, &serde_json::value::RawValue> =
        serde_json::from_slice(&tokenizer_bytes).ok()?;
    let architecture = match descriptor.architecture {
        Architecture::BertForTokenClassification => "BertForTokenClassification",
        Architecture::DebertaV2ForTokenClassification => "DebertaV2ForTokenClassification",
    };
    let model_type = match descriptor.model_type {
        ModelType::Bert => "bert",
        ModelType::DebertaV2 => "deberta-v2",
    };
    let model_limit = config.get("max_position_embeddings")?.as_u64()?;
    let tokenizer_limit = match tokenizer.get("model_max_length") {
        None => model_limit,
        Some(raw) if raw.get() == "null" => model_limit,
        Some(raw) => {
            let raw = raw.get();
            if !raw.bytes().all(|byte| byte.is_ascii_digit()) || raw == "0" {
                return None;
            }
            raw.parse::<u64>().unwrap_or(model_limit).min(model_limit)
        }
    };
    let special = descriptor.special_tokens?;
    let content = tokenizer_limit.checked_sub(special)?;
    let stride = std::cmp::min(
        std::cmp::max(1, tokenizer_limit / 4),
        content.checked_sub(1)?,
    );
    if config.get("architectures")?.as_array()? != &[serde_json::Value::String(architecture.into())]
        || config.get("model_type")?.as_str()? != model_type
        || content < 2
        || tokenizer_limit != descriptor.window_tokens
        || stride != descriptor.stride_tokens
    {
        return None;
    }
    let labels = config.get("id2label")?.as_object()?;
    if labels.is_empty() || labels.keys().any(|key| key.parse::<u64>().is_err()) {
        return None;
    }
    let mut labels = labels
        .values()
        .map(serde_json::Value::as_str)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .filter_map(|label| match label {
            "O" => None,
            label => Some(
                label
                    .strip_prefix("B-")
                    .or_else(|| label.strip_prefix("I-"))
                    .unwrap_or(label),
            ),
        })
        .map(|label| {
            (!generic_label(label))
                .then(|| EntityType::parse(label.into()).ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>()?;
    labels.sort_unstable();
    labels.dedup();
    if !legacy
        && labels
            != descriptor
                .entity_types
                .iter()
                .map(|label| EntityType::parse(label.clone()).ok())
                .collect::<Option<Vec<_>>>()?
    {
        return None;
    }
    (!labels.is_empty()).then_some(labels)
}

fn receipt_matches(bytes: &[u8], descriptor: &ModelDescriptor) -> bool {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        name: String,
        version: String,
        repository: String,
    }
    serde_json::from_slice::<Receipt>(bytes).is_ok_and(|receipt| {
        receipt.name == descriptor.name
            && receipt.version == descriptor.version
            && receipt.repository == descriptor.repository
    })
}
fn installed_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file() && !crate::domain::paths::is_link(metadata))
        .map(|metadata| metadata.len())
        .sum()
}
fn safe_root(root: &Path) -> Result<bool, AppError> {
    crate::domain::paths::check_absolute(root)
        .map_err(|_| AppError::new("invalid_model_manifest"))?;
    let parent = root
        .parent()
        .ok_or_else(|| AppError::new("invalid_model_manifest"))?;
    crate::domain::paths::reject_links(parent)
        .map_err(|_| AppError::new("invalid_model_manifest"))?;
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(AppError::new("invalid_model_manifest")),
        Ok(metadata) if crate::domain::paths::is_link(&metadata) || !metadata.is_dir() => {
            Err(AppError::new("invalid_model_manifest"))
        }
        Ok(_) => {
            crate::domain::paths::reject_links(root)
                .map_err(|_| AppError::new("invalid_model_manifest"))?;
            Ok(true)
        }
    }
}
fn directory_on_disk(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !crate::domain::paths::is_link(&metadata))
}
fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && !crate::domain::paths::is_link(&metadata))
}

fn opaque(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 512
        && !value.chars().any(|character| character.is_control())
}
fn required_fields(value: &serde_json::Value, fields: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| fields.iter().all(|field| object.contains_key(*field)))
}
fn hash(value: &str, git: bool) -> bool {
    let length = if git { 40 } else { 64 };
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn repository(value: &str) -> bool {
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 2 && parts.into_iter().all(repository_component)
}
fn huggingface_url(value: &str) -> bool {
    let Some(name) = value.strip_prefix("https://huggingface.co/") else {
        return false;
    };
    let name = name.strip_suffix('/').unwrap_or(name);
    !name.is_empty()
        && (value == format!("https://huggingface.co/{name}")
            || value == format!("https://huggingface.co/{name}/"))
        && repository(name)
}
fn uuid(value: &str) -> bool {
    value
        .parse::<uuid::Uuid>()
        .is_ok_and(|parsed| parsed.hyphenated().to_string() == value)
}
fn repository_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        && !value.ends_with(['.', '-'])
        && !value.contains("..")
        && !value.contains("--")
}
fn entity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}
fn generic_label(value: &str) -> bool {
    value.strip_prefix("LABEL_").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}
fn windows_reserved(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
}
fn filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !value.ends_with('.')
        && !windows_reserved(value)
}
fn directory(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !windows_reserved(value)
}
fn legacy_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.starts_with('/')
        && value.as_bytes().get(1).is_none_or(|byte| *byte != b':')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(part) if !part.is_empty()))
}

/// A read-only lease on the immutable installation receipt. No lock file is created.
#[derive(Debug)]
pub struct ModelUseGuard {
    _receipt: fs::File,
}
impl ModelUseGuard {
    pub fn acquire(root: &Path, name: &str) -> Result<Self, AppError> {
        use std::io::Read;
        let catalog = catalog_models()?;
        let (registry, legacy) = read_store(root, &catalog)?;
        let record = registry
            .models
            .iter()
            .find(|record| record.descriptor.name == name && record.state == ModelState::Ready)
            .ok_or_else(|| AppError::new("model_not_found"))?;
        let model = root.join(
            record
                .path
                .as_ref()
                .ok_or_else(|| AppError::new("model_not_found"))?,
        );
        crate::domain::paths::reject_links(&model)
            .map_err(|_| AppError::new("model_path_unsafe"))?;
        let path = model.join("redactio-model.json");
        if !regular_file(&path) {
            return Err(AppError::new("model_incompatible"));
        }
        let mut receipt = fs::File::open(&path).map_err(|_| AppError::new("model_not_found"))?;
        receipt
            .try_lock_shared()
            .map_err(|_| AppError::new("model_in_use"))?;
        let mut bytes = Vec::new();
        (&mut receipt)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| AppError::new("model_incompatible"))?;
        let held = crate::domain::storage::snapshot_file(&receipt)
            .map_err(|_| AppError::new("model_path_unsafe"))?;
        let named = crate::domain::storage::file_snapshot(&path)
            .map_err(|_| AppError::new("model_path_unsafe"))?;
        let (current, current_legacy) = read_store(root, &catalog)?;
        if held != named
            || bytes.len() > 4096
            || !receipt_matches(&bytes, &record.descriptor)
            || !current.models.contains(record)
            || current_legacy != legacy
            || compatibility(root, record, legacy, &catalog).is_none()
        {
            return Err(AppError::new("model_incompatible"));
        }
        Ok(Self { _receipt: receipt })
    }
}

impl Drop for ModelUseGuard {
    fn drop(&mut self) {
        let _ = self._receipt.unlock();
    }
}
