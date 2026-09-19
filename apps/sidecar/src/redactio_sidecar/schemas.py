from __future__ import annotations

import re
from datetime import datetime
from typing import Annotated, Literal, TypeAlias
from uuid import UUID

from pydantic import (
    AfterValidator,
    AwareDatetime,
    BaseModel,
    BeforeValidator,
    ConfigDict,
    Field,
    StringConstraints,
    field_validator,
    model_validator,
)


def _canonical_uuid(value: str) -> str:
    try:
        parsed = UUID(value)
    except (AttributeError, TypeError, ValueError) as error:
        raise ValueError("invalid UUID") from error
    if str(parsed) != value:
        raise ValueError("UUID must use canonical lowercase hyphenated form")
    return value


_RFC3339 = re.compile(
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})$"
)


def _parse_rfc3339(value: object) -> datetime:
    if isinstance(value, datetime):
        return value
    if not isinstance(value, str) or _RFC3339.fullmatch(value) is None:
        raise ValueError("timestamp must be an RFC-3339 string")
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("invalid RFC-3339 timestamp") from error


UuidString = Annotated[str, AfterValidator(_canonical_uuid)]
RequestId = Annotated[str, StringConstraints(min_length=1, max_length=128)]
OpaqueId = Annotated[str, StringConstraints(min_length=1, max_length=128)]
NonEmptyString = Annotated[str, StringConstraints(min_length=1)]
SafeCode = Annotated[
    str,
    StringConstraints(min_length=1, max_length=128, pattern=r"^[a-z][a-z0-9_]*$"),
]
Sha256 = Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]
Offset = Annotated[int, Field(strict=True, ge=0)]
Confidence = Annotated[
    float,
    Field(strict=True, ge=0.0, le=1.0, allow_inf_nan=False),
]
Timestamp = Annotated[AwareDatetime, BeforeValidator(_parse_rfc3339)]

EntityType: TypeAlias = Literal[
    "PERSON",
    "LOCATION",
    "EMAIL_ADDRESS",
    "PHONE_NUMBER",
    "IBAN_CODE",
    "IP_ADDRESS",
    "URL",
    "DATE_TIME",
    "CUSTOM",
]
ReviewStatus: TypeAlias = Literal["pending", "approved", "rejected", "needs-rework"]

DEFAULT_ENTITIES: tuple[EntityType, ...] = (
    "PERSON",
    "LOCATION",
    "EMAIL_ADDRESS",
    "PHONE_NUMBER",
    "IBAN_CODE",
    "IP_ADDRESS",
    "URL",
    "DATE_TIME",
)


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class EmptyPayload(StrictModel):
    pass


class RegexRule(StrictModel):
    id: UuidString
    entity_type: EntityType
    enabled: bool
    kind: Literal["regex"]
    pattern: NonEmptyString

    @field_validator("pattern")
    @classmethod
    def pattern_must_compile(cls, value: str) -> str:
        try:
            re.compile(value)
        except re.error as error:
            raise ValueError("invalid regular expression") from error
        return value


class WordRule(StrictModel):
    id: UuidString
    entity_type: EntityType
    enabled: bool
    kind: Literal["words"]
    words: list[NonEmptyString] = Field(min_length=1)


CustomRule: TypeAlias = Annotated[RegexRule | WordRule, Field(discriminator="kind")]


class ProcessingConfig(StrictModel):
    model: NonEmptyString = "de_core_news_lg"
    enabled_entities: list[EntityType] = Field(default_factory=lambda: list(DEFAULT_ENTITIES))
    custom_rules: list[CustomRule] = Field(default_factory=list)
    include_positions: bool = True

    @model_validator(mode="after")
    def values_are_unique(self) -> ProcessingConfig:
        if len(set(self.enabled_entities)) != len(self.enabled_entities):
            raise ValueError("enabled entities must be unique")
        rule_ids = [rule.id for rule in self.custom_rules]
        if len(set(rule_ids)) != len(rule_ids):
            raise ValueError("custom rule IDs must be unique")
        return self


class Detection(StrictModel):
    id: OpaqueId
    start: Offset
    end: Offset
    entity_type: EntityType
    confidence: Confidence | None
    recognizer: NonEmptyString
    origin: Literal["automatic", "manual"]

    @model_validator(mode="after")
    def end_follows_start(self) -> Detection:
        if self.start >= self.end:
            raise ValueError("detection end must follow start")
        return self


class Decisions(StrictModel):
    dismissed_ids: list[OpaqueId] = Field(default_factory=list)
    manual: list[Detection] = Field(default_factory=list)


class OutputEntry(StrictModel):
    start_offset: Offset
    end_offset: Offset
    entity_type: EntityType
    placeholder: NonEmptyString
    confidence: Confidence | None
    recognizer: NonEmptyString
    origin: Literal["automatic", "manual", "merged"]

    @model_validator(mode="after")
    def end_follows_start(self) -> OutputEntry:
        if self.start_offset >= self.end_offset:
            raise ValueError("output end must follow start")
        return self


class EngineInfo(StrictModel):
    engine_version: NonEmptyString
    model_name: NonEmptyString
    model_version: NonEmptyString
    recognizers: list[NonEmptyString]
    extraction_version: NonEmptyString


class DocumentKey(StrictModel):
    sync_pair_id: UuidString
    doc_id: str

    @field_validator("doc_id")
    @classmethod
    def valid_document_id(cls, value: str) -> str:
        if re.fullmatch(r"doc-[0-9]{4,}", value) is None or int(value[4:]) == 0:
            raise ValueError("invalid document ID")
        return value


class DocumentMeta(DocumentKey):
    source_hash_sha256: Sha256
    processing_revision: UuidString
    redacted_at: Timestamp


class ProcessRequest(DocumentMeta):
    source_path: NonEmptyString


class ReviewRequest(ProcessRequest):
    detections: list[Detection]
    decisions: Decisions
    review_status: ReviewStatus
    reviewed_at: Timestamp | None
    acknowledged_warnings: list[SafeCode]


class ProcessResult(DocumentMeta):
    markdown: str
    body: str
    original_text: str
    detections: list[Detection]
    redactions: list[OutputEntry]
    warnings: list[SafeCode]
    body_was_empty: bool
    review_status: ReviewStatus
    engine: EngineInfo


class CollectionPayload(StrictModel):
    sync_pair_id: UuidString
    processing_revision: UuidString


class ConfigurePayload(CollectionPayload):
    config: ProcessingConfig


class ConfigureResultPayload(CollectionPayload):
    engine: EngineInfo


class PreviewRulesPayload(CollectionPayload):
    text: str


class PreviewRulesResultPayload(CollectionPayload):
    detections: list[Detection]


class PingResultPayload(StrictModel):
    protocol_version: Literal[1]


class SafeError(StrictModel):
    code: SafeCode
    retryable: bool = False


class PingRequest(StrictModel):
    id: RequestId
    type: Literal["ping"]
    payload: EmptyPayload


class ConfigureRequest(StrictModel):
    id: RequestId
    type: Literal["configure"]
    payload: ConfigurePayload


class ProcessDocumentRequest(StrictModel):
    id: RequestId
    type: Literal["process_document"]
    payload: ProcessRequest


class PreviewRulesRequest(StrictModel):
    id: RequestId
    type: Literal["preview_rules"]
    payload: PreviewRulesPayload


class RenderReviewRequest(StrictModel):
    id: RequestId
    type: Literal["render_review"]
    payload: ReviewRequest


Request: TypeAlias = Annotated[
    PingRequest
    | ConfigureRequest
    | ProcessDocumentRequest
    | PreviewRulesRequest
    | RenderReviewRequest,
    Field(discriminator="type"),
]


class PingResponse(StrictModel):
    id: RequestId
    type: Literal["ping_result"]
    payload: PingResultPayload


class ConfigureResponse(StrictModel):
    id: RequestId
    type: Literal["configure_result"]
    payload: ConfigureResultPayload


class ProcessDocumentResponse(StrictModel):
    id: RequestId
    type: Literal["process_document_result"]
    payload: ProcessResult


class PreviewRulesResponse(StrictModel):
    id: RequestId
    type: Literal["preview_rules_result"]
    payload: PreviewRulesResultPayload


class RenderReviewResponse(StrictModel):
    id: RequestId
    type: Literal["render_review_result"]
    payload: ProcessResult


class ErrorResponse(StrictModel):
    id: Annotated[str, StringConstraints(max_length=128)]
    type: Literal["error"]
    payload: SafeError


Response: TypeAlias = Annotated[
    PingResponse
    | ConfigureResponse
    | ProcessDocumentResponse
    | PreviewRulesResponse
    | RenderReviewResponse
    | ErrorResponse,
    Field(discriminator="type"),
]
