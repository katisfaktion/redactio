from __future__ import annotations

from collections import defaultdict
from datetime import datetime
from statistics import fmean
from uuid import UUID

import yaml

from .schemas import (
    DocumentMeta,
    EngineInfo,
    EntityType,
    NonEmptyString,
    OutputEntry,
    ReviewStatus,
    SafeCode,
    StrictModel,
)

PUBLIC_RECOGNIZERS = frozenset(
    {
        "SpacyRecognizer",
        "BiomedBertRecognizer",
        "HuggingLilRecognizer",
        "EmailRecognizer",
        "PhoneRecognizer",
        "IbanRecognizer",
        "IpRecognizer",
        "UrlRecognizer",
        "DateRecognizer",
    }
)


def _public_recognizer(value: str) -> bool:
    if value in PUBLIC_RECOGNIZERS:
        return True
    try:
        return str(UUID(value)) == value
    except ValueError:
        return False


class _NlpModel(StrictModel):
    name: NonEmptyString
    version: NonEmptyString


class _Summary(StrictModel):
    count: int
    confidence_min: float | None
    confidence_mean: float | None


class _Frontmatter(StrictModel):
    schema_version: int
    sync_pair_id: str
    doc_id: str
    source_hash_sha256: str
    processing_revision: str
    redacted_at: datetime
    redaction_engine: NonEmptyString
    nlp_model: _NlpModel
    recognizers_used: list[NonEmptyString]
    redactions: list[OutputEntry] | None
    redaction_summary: dict[EntityType, _Summary]
    review_status: ReviewStatus
    reviewed_at: datetime | None
    warnings: list[SafeCode]


def _summary(entries: list[OutputEntry]) -> dict[EntityType, _Summary]:
    by_type: dict[EntityType, list[OutputEntry]] = defaultdict(list)
    for entry in entries:
        by_type[entry.entity_type].append(entry)

    summary: dict[EntityType, _Summary] = {}
    for entity_type in sorted(by_type):
        grouped = by_type[entity_type]
        confidences = [entry.confidence for entry in grouped if entry.confidence is not None]
        summary[entity_type] = _Summary(
            count=len(grouped),
            confidence_min=min(confidences) if confidences else None,
            confidence_mean=fmean(confidences) if confidences else None,
        )
    return summary


def normalize_body(body: str) -> str:
    return body if body.endswith("\n") else body + "\n"


def render_document(
    meta: DocumentMeta,
    engine: EngineInfo,
    body: str,
    entries: list[OutputEntry],
    warnings: list[str],
    status: ReviewStatus,
    reviewed_at: datetime | None,
    include_positions: bool,
) -> str:
    if any(not _public_recognizer(recognizer) for recognizer in engine.recognizers):
        raise ValueError("unsafe recognizer metadata")
    allowed_recognizers = {*engine.recognizers, "manual", "merged"}
    if any(entry.recognizer not in allowed_recognizers for entry in entries):
        raise ValueError("unsafe recognizer metadata")

    frontmatter = _Frontmatter(
        schema_version=1,
        sync_pair_id=meta.sync_pair_id,
        doc_id=meta.doc_id,
        source_hash_sha256=meta.source_hash_sha256,
        processing_revision=meta.processing_revision,
        redacted_at=meta.redacted_at,
        redaction_engine=engine.engine_version,
        nlp_model=_NlpModel(name=engine.model_name, version=engine.model_version),
        recognizers_used=engine.recognizers,
        redactions=entries if include_positions else None,
        redaction_summary=_summary(entries),
        review_status=status,
        reviewed_at=reviewed_at,
        warnings=warnings,
    )

    yaml_document = yaml.safe_dump(
        frontmatter.model_dump(
            mode="json", exclude={"redactions"} if not include_positions else set()
        ),
        allow_unicode=True,
        sort_keys=False,
    )
    return f"---\n{yaml_document}---\n{normalize_body(body)}"
