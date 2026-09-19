from datetime import UTC, datetime
from uuid import uuid4

import yaml

from redactio_sidecar.frontmatter import render_document
from redactio_sidecar.schemas import DocumentMeta, EngineInfo, OutputEntry


def metadata() -> DocumentMeta:
    return DocumentMeta(
        sync_pair_id=str(uuid4()),
        doc_id="doc-0001",
        source_hash_sha256="0" * 64,
        processing_revision=str(uuid4()),
        redacted_at=datetime(2026, 9, 19, 8, 0, tzinfo=UTC),
    )


def engine_info() -> EngineInfo:
    return EngineInfo(
        engine_version="test",
        model_name="de_core_news_lg",
        model_version="1",
        recognizers=["SpacyRecognizer", "EmailRecognizer"],
        extraction_version="1",
    )


def entry(
    entity_type: str,
    confidence: float | None,
    recognizer: str,
    origin: str,
) -> OutputEntry:
    return OutputEntry.model_validate(
        {
            "start_offset": 0,
            "end_offset": 10,
            "entity_type": entity_type,
            "placeholder": f"<{entity_type}_1>",
            "confidence": confidence,
            "recognizer": recognizer,
            "origin": origin,
        }
    )


def parse_frontmatter(markdown: str) -> dict[str, object]:
    return yaml.safe_load(markdown.split("---", 2)[1])


def test_metadata_is_structured_and_positions_can_be_omitted() -> None:
    meta = metadata()

    markdown = render_document(meta, engine_info(), "Hello", [], [], "pending", None, False)
    frontmatter = parse_frontmatter(markdown)

    assert list(frontmatter) == [
        "schema_version",
        "sync_pair_id",
        "doc_id",
        "source_hash_sha256",
        "processing_revision",
        "redacted_at",
        "redaction_engine",
        "nlp_model",
        "recognizers_used",
        "redaction_summary",
        "review_status",
        "reviewed_at",
        "warnings",
    ]
    assert frontmatter["doc_id"] == "doc-0001"
    assert frontmatter["redacted_at"] == "2026-09-19T08:00:00Z"
    assert frontmatter["redaction_engine"] == "test"
    assert frontmatter["nlp_model"] == {"name": "de_core_news_lg", "version": "1"}
    assert frontmatter["recognizers_used"] == ["SpacyRecognizer", "EmailRecognizer"]
    assert frontmatter["redaction_summary"] == {}
    assert "redactions" not in frontmatter
    assert "review_notes" not in frontmatter
    assert markdown.endswith("Hello\n")


def test_summary_counts_occurrences_and_only_available_confidence() -> None:
    entries = [
        entry("PERSON", 0.8, "SpacyRecognizer", "automatic"),
        entry("PERSON", None, "manual", "manual"),
        entry("LOCATION", None, "manual", "manual"),
    ]

    frontmatter = parse_frontmatter(
        render_document(
            metadata(),
            engine_info(),
            "<PERSON_1>",
            entries,
            ["comments"],
            "needs-rework",
            datetime(2026, 9, 19, 9, 30, tzinfo=UTC),
            True,
        )
    )

    assert frontmatter["recognizers_used"] == ["SpacyRecognizer", "EmailRecognizer"]
    assert frontmatter["redaction_summary"] == {
        "LOCATION": {"count": 1, "confidence_min": None, "confidence_mean": None},
        "PERSON": {"count": 2, "confidence_min": 0.8, "confidence_mean": 0.8},
    }
    assert frontmatter["reviewed_at"] == "2026-09-19T09:30:00Z"
    assert frontmatter["warnings"] == ["comments"]
    assert frontmatter["redactions"] == [item.model_dump() for item in entries]


def test_yaml_delimiter_like_body_is_preserved_verbatim() -> None:
    body = "---\nPatient: Käthe\n..."

    markdown = render_document(metadata(), engine_info(), body, [], [], "pending", None, True)

    assert markdown.partition("\n---\n")[2] == body + "\n"


def test_biomedbert_recognizer_identity_is_safe_to_render() -> None:
    info = engine_info().model_copy(update={"recognizers": ["BiomedBertRecognizer"]})
    rendered = render_document(
        metadata(),
        info,
        "<PERSON_1>",
        [
            entry("PERSON", 0.9, "BiomedBertRecognizer", "automatic"),
        ],
        [],
        "pending",
        None,
        True,
    )
    assert parse_frontmatter(rendered)["recognizers_used"] == ["BiomedBertRecognizer"]


def test_private_recognizer_metadata_is_rejected() -> None:
    unsafe = entry("PERSON", 0.8, "Patient Käthe", "automatic")

    try:
        render_document(
            metadata(), engine_info(), "<PERSON_1>", [unsafe], [], "pending", None, True
        )
    except ValueError as error:
        assert str(error) == "unsafe recognizer metadata"
    else:
        raise AssertionError("private recognizer metadata was rendered")

    unsafe_engine = engine_info().model_copy(update={"recognizers": ["Patient Käthe"]})
    try:
        render_document(metadata(), unsafe_engine, "Hello", [], [], "pending", None, True)
    except ValueError as error:
        assert str(error) == "unsafe recognizer metadata"
    else:
        raise AssertionError("private configured recognizer metadata was rendered")
