import json
import math
import subprocess
import sys
from copy import deepcopy
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import pytest
from pydantic import TypeAdapter, ValidationError

from redactio_sidecar.schemas import (
    Decisions,
    Detection,
    ProcessingConfig,
    Request,
    Response,
)

PAIR_ID = "11111111-1111-4111-8111-111111111111"
REVISION = "22222222-2222-4222-8222-222222222222"
RULE_ID = "33333333-3333-4333-8333-333333333333"
TIMESTAMP = "2026-09-19T10:00:00+02:00"
REQUEST_ADAPTER = TypeAdapter(Request)
RESPONSE_ADAPTER = TypeAdapter(Response)


def engine_info() -> dict[str, Any]:
    return {
        "engine_version": "synthetic-engine",
        "model_name": "de_core_news_lg",
        "model_version": "synthetic-model",
        "recognizers": ["synthetic-recognizer"],
        "extraction_version": "synthetic-extractor",
    }


def document_meta() -> dict[str, Any]:
    return {
        "sync_pair_id": PAIR_ID,
        "doc_id": "doc-0001",
        "source_hash_sha256": "0" * 64,
        "processing_revision": REVISION,
        "redacted_at": TIMESTAMP,
    }


def detection() -> dict[str, Any]:
    return {
        "id": "detection-1",
        "start": 0,
        "end": 4,
        "entity_type": "PERSON",
        "confidence": 0.9,
        "recognizer": "synthetic-recognizer",
        "origin": "automatic",
    }


def process_result_payload() -> dict[str, Any]:
    return {
        **document_meta(),
        "markdown": "---\nschema_version: 1\n---\n<PERSON_1>",
        "body": "<PERSON_1>",
        "original_text": "Anna",
        "detections": [detection()],
        "redactions": [
            {
                "start_offset": 0,
                "end_offset": 10,
                "entity_type": "PERSON",
                "placeholder": "<PERSON_1>",
                "confidence": 0.9,
                "recognizer": "synthetic-recognizer",
                "origin": "automatic",
            }
        ],
        "warnings": [],
        "body_was_empty": False,
        "review_status": "pending",
        "engine": engine_info(),
    }


def synthetic_messages() -> list[dict[str, Any]]:
    config = {
        "model": "de_core_news_lg",
        "enabled_entities": [
            "PERSON",
            "LOCATION",
            "EMAIL_ADDRESS",
            "PHONE_NUMBER",
            "IBAN_CODE",
            "IP_ADDRESS",
            "URL",
            "DATE_TIME",
        ],
        "custom_rules": [
            {
                "id": RULE_ID,
                "entity_type": "CUSTOM",
                "enabled": True,
                "kind": "regex",
                "pattern": "Synthetic_[0-9]+",
            },
            {
                "id": "44444444-4444-4444-8444-444444444444",
                "entity_type": "CUSTOM",
                "enabled": True,
                "kind": "words",
                "words": ["Synthetic"],
            },
        ],
        "include_positions": True,
    }
    review_payload = {
        **document_meta(),
        "source_path": "synthetic.docx",
        "detections": [detection()],
        "decisions": {"dismissed_ids": [], "manual": []},
        "review_status": "pending",
        "reviewed_at": None,
        "acknowledged_warnings": [],
    }
    return [
        {"id": "ping", "type": "ping", "payload": {}},
        {
            "id": "configure",
            "type": "configure",
            "payload": {
                "sync_pair_id": PAIR_ID,
                "processing_revision": REVISION,
                "config": config,
            },
        },
        {
            "id": "process",
            "type": "process_document",
            "payload": {**document_meta(), "source_path": "synthetic.docx"},
        },
        {
            "id": "preview",
            "type": "preview_rules",
            "payload": {
                "sync_pair_id": PAIR_ID,
                "processing_revision": REVISION,
                "text": "Synthetic_1",
            },
        },
        {"id": "render", "type": "render_review", "payload": review_payload},
        {
            "id": "ping",
            "type": "ping_result",
            "payload": {"protocol_version": 1},
        },
        {
            "id": "configure",
            "type": "configure_result",
            "payload": {
                "sync_pair_id": PAIR_ID,
                "processing_revision": REVISION,
                "engine": engine_info(),
            },
        },
        {
            "id": "process",
            "type": "process_document_result",
            "payload": process_result_payload(),
        },
        {
            "id": "preview",
            "type": "preview_rules_result",
            "payload": {
                "sync_pair_id": PAIR_ID,
                "processing_revision": REVISION,
                "detections": [detection()],
            },
        },
        {
            "id": "render",
            "type": "render_review_result",
            "payload": process_result_payload(),
        },
        {
            "id": "error",
            "type": "error",
            "payload": {"code": "synthetic_error", "retryable": False},
        },
    ]


def test_processing_defaults_and_empty_decisions_match_shared_contract():
    config = ProcessingConfig()
    assert config.model == "de_core_news_lg"
    assert config.enabled_entities == [
        "PERSON",
        "LOCATION",
        "EMAIL_ADDRESS",
        "PHONE_NUMBER",
        "IBAN_CODE",
        "IP_ADDRESS",
        "URL",
        "DATE_TIME",
    ]
    assert config.custom_rules == []
    assert config.include_positions is True
    assert Decisions().model_dump() == {"dismissed_ids": [], "manual": []}


@pytest.mark.parametrize(
    "mutation",
    [
        lambda value: value.update(sync_pair_id="not-a-uuid"),
        lambda value: value.update(processing_revision="not-a-uuid"),
        lambda value: value.update(doc_id="doc-1"),
        lambda value: value.update(source_hash_sha256="A" * 64),
        lambda value: value.update(redacted_at="2026-09-19T10:00:00"),
        lambda value: value.update(extra="private"),
    ],
)
def test_document_metadata_rejects_invalid_fields(mutation):
    payload = document_meta()
    mutation(payload)
    message = {
        "id": "invalid",
        "type": "process_document",
        "payload": {**payload, "source_path": "synthetic.docx"},
    }
    with pytest.raises(ValidationError):
        REQUEST_ADAPTER.validate_python(message)


@pytest.mark.parametrize(
    "mutation",
    [
        lambda value: value.update(start=-1),
        lambda value: value.update(end=0),
        lambda value: value.update(entity_type="UNKNOWN"),
        lambda value: value.update(confidence=-0.1),
        lambda value: value.update(confidence=1.1),
        lambda value: value.update(confidence=math.nan),
        lambda value: value.update(extra="private"),
    ],
)
def test_detection_rejects_invalid_spans_confidence_and_fields(mutation):
    value = detection()
    mutation(value)
    with pytest.raises(ValidationError):
        Detection.model_validate(value)


@pytest.mark.parametrize("confidence", [True, "0.5"])
def test_detection_rejects_non_numeric_json_confidence(confidence):
    value = detection()
    value["confidence"] = confidence
    with pytest.raises(ValidationError):
        Detection.model_validate(value)


@pytest.mark.parametrize("field", ["enabled", "include_positions"])
def test_processing_config_rejects_coerced_json_booleans(field):
    value: dict[str, Any] = {"include_positions": True}
    if field == "enabled":
        value["custom_rules"] = [
            {
                "id": RULE_ID,
                "entity_type": "CUSTOM",
                "enabled": "false",
                "kind": "words",
                "words": ["Synthetic"],
            }
        ]
    else:
        value[field] = "false"
    with pytest.raises(ValidationError):
        ProcessingConfig.model_validate(value)


@pytest.mark.parametrize("timestamp", [0, "2026-09-19 10:00:00+02:00"])
def test_document_metadata_requires_rfc3339_timestamp_string(timestamp):
    payload = document_meta()
    payload["redacted_at"] = timestamp
    message = {
        "id": "invalid-time",
        "type": "process_document",
        "payload": {**payload, "source_path": "synthetic.docx"},
    }
    with pytest.raises(ValidationError):
        REQUEST_ADAPTER.validate_python(message)


def test_internal_model_construction_accepts_aware_datetime():
    payload = document_meta()
    payload["redacted_at"] = datetime(2026, 9, 19, 8, 0, tzinfo=UTC)
    REQUEST_ADAPTER.validate_python(
        {
            "id": "internal-time",
            "type": "process_document",
            "payload": {**payload, "source_path": "synthetic.docx"},
        }
    )


@pytest.mark.parametrize(
    "rule",
    [
        {
            "id": RULE_ID,
            "entity_type": "CUSTOM",
            "enabled": True,
            "kind": "regex",
            "pattern": "(",
        },
        {
            "id": RULE_ID,
            "entity_type": "CUSTOM",
            "enabled": True,
            "kind": "words",
            "words": [""],
        },
    ],
)
def test_custom_rules_reject_invalid_regex_and_empty_words(rule):
    with pytest.raises(ValidationError):
        ProcessingConfig(custom_rules=[rule])


def test_processing_config_rejects_duplicate_rule_ids():
    rule = {
        "id": RULE_ID,
        "entity_type": "CUSTOM",
        "enabled": True,
        "kind": "words",
        "words": ["Synthetic"],
    }
    with pytest.raises(ValidationError):
        ProcessingConfig(custom_rules=[rule, deepcopy(rule)])


def test_all_synthetic_contract_messages_validate_and_reject_extra_fields():
    messages = synthetic_messages()
    for message in messages[:5]:
        REQUEST_ADAPTER.validate_python(message)
    for message in messages[5:]:
        RESPONSE_ADAPTER.validate_python(message)

    invalid = deepcopy(messages[0])
    invalid["payload"]["extra"] = "private"
    with pytest.raises(ValidationError):
        REQUEST_ADAPTER.validate_python(invalid)


@pytest.mark.parametrize("message_index,required_field", [(1, "config"), (4, "decisions")])
def test_request_payloads_require_all_shared_contract_fields(message_index, required_field):
    invalid = deepcopy(synthetic_messages()[message_index])
    del invalid["payload"][required_field]
    with pytest.raises(ValidationError):
        REQUEST_ADAPTER.validate_python(invalid)


def test_emit_cli_outputs_only_valid_jsonl_contract_objects():
    completed = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), "--emit"],
        check=True,
        capture_output=True,
        text=True,
    )
    emitted = [json.loads(line) for line in completed.stdout.splitlines()]
    assert emitted == synthetic_messages()


if __name__ == "__main__" and sys.argv[1:] == ["--emit"]:
    for synthetic_message in synthetic_messages():
        print(json.dumps(synthetic_message, separators=(",", ":")))
