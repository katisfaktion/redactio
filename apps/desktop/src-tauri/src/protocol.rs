use crate::domain::settings::{EntityType, ProcessingConfig};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

/// Full identity shared by private review records and document commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentKey {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "document_id")]
    pub doc_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineInfo {
    #[serde(deserialize_with = "nonempty")]
    pub engine_version: String,
    #[serde(deserialize_with = "nonempty")]
    pub model_name: String,
    #[serde(deserialize_with = "nonempty")]
    pub model_version: String,
    #[serde(deserialize_with = "nonempty_list")]
    pub recognizers: Vec<String>,
    #[serde(deserialize_with = "nonempty")]
    pub extraction_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInfo {
    #[serde(deserialize_with = "nonempty")]
    pub name: String,
    #[serde(deserialize_with = "nonempty")]
    pub version: String,
    pub compatible: bool,
    pub entity_types: Vec<EntityType>,
}

#[derive(Debug, Clone, PartialEq, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectionOrigin {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputOrigin {
    Automatic,
    Manual,
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy, Serialize, Deserialize)]
pub enum ReviewStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "approved")]
    Approved,
    #[serde(rename = "rejected")]
    Rejected,
    #[serde(rename = "needs-rework")]
    NeedsRework,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Detection {
    pub id: String,
    pub start: u64,
    pub end: u64,
    pub entity_type: EntityType,
    pub confidence: Option<f64>,
    pub recognizer: String,
    pub origin: DetectionOrigin,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DetectionWire {
    id: String,
    start: u64,
    end: u64,
    entity_type: EntityType,
    #[serde(deserialize_with = "required_confidence")]
    confidence: Option<f64>,
    recognizer: String,
    origin: DetectionOrigin,
}

impl<'de> Deserialize<'de> for Detection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = DetectionWire::deserialize(deserializer)?;
        if value.id.is_empty()
            || value.id.len() > 128
            || value.recognizer.is_empty()
            || value.start >= value.end
            || value
                .confidence
                .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
        {
            return Err(D::Error::custom("invalid detection"));
        }
        Ok(Self {
            id: value.id,
            start: value.start,
            end: value.end,
            entity_type: value.entity_type,
            confidence: value.confidence,
            recognizer: value.recognizer,
            origin: value.origin,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decisions {
    #[serde(default, deserialize_with = "opaque_ids")]
    pub dismissed_ids: Vec<String>,
    #[serde(default)]
    pub manual: Vec<Detection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputEntry {
    pub start_offset: u64,
    pub end_offset: u64,
    pub entity_type: EntityType,
    pub placeholder: String,
    pub confidence: Option<f64>,
    pub recognizer: String,
    pub origin: OutputOrigin,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputEntryWire {
    start_offset: u64,
    end_offset: u64,
    entity_type: EntityType,
    placeholder: String,
    #[serde(deserialize_with = "required_confidence")]
    confidence: Option<f64>,
    recognizer: String,
    origin: OutputOrigin,
}

impl<'de> Deserialize<'de> for OutputEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = OutputEntryWire::deserialize(deserializer)?;
        if value.start_offset >= value.end_offset
            || value.placeholder.is_empty()
            || value.recognizer.is_empty()
            || value
                .confidence
                .is_some_and(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
        {
            return Err(D::Error::custom("invalid output entry"));
        }
        Ok(Self {
            start_offset: value.start_offset,
            end_offset: value.end_offset,
            entity_type: value.entity_type,
            placeholder: value.placeholder,
            confidence: value.confidence,
            recognizer: value.recognizer,
            origin: value.origin,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurePayload {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    pub config: ProcessingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureResult {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    pub engine: EngineInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRulesPayload {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRulesResult {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    pub detections: Vec<Detection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingResult {
    #[serde(deserialize_with = "protocol_version")]
    pub protocol_version: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessRequest {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "document_id")]
    pub doc_id: String,
    #[serde(deserialize_with = "sha256")]
    pub source_hash_sha256: String,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    #[serde(deserialize_with = "timestamp")]
    pub redacted_at: String,
    #[serde(deserialize_with = "nonempty")]
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "document_id")]
    pub doc_id: String,
    #[serde(deserialize_with = "sha256")]
    pub source_hash_sha256: String,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    #[serde(deserialize_with = "timestamp")]
    pub redacted_at: String,
    #[serde(deserialize_with = "nonempty")]
    pub source_path: String,
    pub detections: Vec<Detection>,
    pub decisions: Decisions,
    pub review_status: ReviewStatus,
    #[serde(deserialize_with = "optional_timestamp")]
    pub reviewed_at: Option<String>,
    #[serde(deserialize_with = "safe_codes")]
    pub acknowledged_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessResult {
    #[serde(deserialize_with = "canonical_uuid")]
    pub sync_pair_id: Uuid,
    #[serde(deserialize_with = "document_id")]
    pub doc_id: String,
    #[serde(deserialize_with = "sha256")]
    pub source_hash_sha256: String,
    #[serde(deserialize_with = "canonical_uuid")]
    pub processing_revision: Uuid,
    #[serde(deserialize_with = "timestamp")]
    pub redacted_at: String,
    pub markdown: String,
    pub body: String,
    pub original_text: String,
    pub detections: Vec<Detection>,
    pub redactions: Vec<OutputEntry>,
    #[serde(deserialize_with = "safe_codes")]
    pub warnings: Vec<String>,
    pub body_was_empty: bool,
    pub review_status: ReviewStatus,
    pub engine: EngineInfo,
}

pub(crate) fn canonical_uuid<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let value = String::deserialize(deserializer)?;
    let parsed = Uuid::parse_str(&value).map_err(D::Error::custom)?;
    if parsed.to_string() != value {
        return Err(D::Error::custom("UUID is not canonical"));
    }
    Ok(parsed)
}

fn nonempty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        return Err(D::Error::custom("string is empty"));
    }
    Ok(value)
}

fn nonempty_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let values = Vec::<String>::deserialize(deserializer)?;
    if values.iter().any(String::is_empty) {
        return Err(D::Error::custom("string is empty"));
    }
    Ok(values)
}

fn document_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    let digits = value.strip_prefix("doc-").unwrap_or_default();
    if digits.len() < 4
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || digits.bytes().all(|byte| byte == b'0')
    {
        return Err(D::Error::custom("invalid document ID"));
    }
    Ok(value)
}

fn protocol_version<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let value = u8::deserialize(deserializer)?;
    if value != 1 {
        return Err(D::Error::custom("unsupported protocol version"));
    }
    Ok(value)
}

fn required_confidence<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    Option::<f64>::deserialize(deserializer)
}

pub(crate) fn sha256<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(D::Error::custom("invalid SHA-256"));
    }
    Ok(value)
}

pub(crate) fn timestamp<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    validate_timestamp(&value).map_err(D::Error::custom)?;
    Ok(value)
}

pub(crate) fn optional_timestamp<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    if let Some(value) = value.as_deref() {
        validate_timestamp(value).map_err(D::Error::custom)?;
    }
    Ok(value)
}

fn validate_timestamp(value: &str) -> Result<(), &'static str> {
    if value.len() < 20 || !matches!(value.as_bytes().get(10), Some(b'T' | b't')) {
        return Err("invalid RFC-3339 timestamp");
    }
    let mut normalized = value.to_owned();
    normalized.replace_range(10..11, "T");
    if normalized.ends_with('z') {
        normalized.replace_range(normalized.len() - 1.., "Z");
    }
    OffsetDateTime::parse(&normalized, &Rfc3339)
        .map(|_| ())
        .map_err(|_| "invalid RFC-3339 timestamp")
}

fn opaque_ids<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let values = Vec::<String>::deserialize(deserializer)?;
    if values
        .iter()
        .any(|value| value.is_empty() || value.len() > 128)
    {
        return Err(D::Error::custom("invalid opaque ID"));
    }
    Ok(values)
}

pub(crate) fn safe_codes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let values = Vec::<String>::deserialize(deserializer)?;
    if values.iter().any(|value| {
        value.is_empty()
            || value.len() > 128
            || !value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase() || (index > 0 && (byte.is_ascii_digit() || byte == b'_'))
            })
    }) {
        return Err(D::Error::custom("invalid safe code"));
    }
    Ok(values)
}
