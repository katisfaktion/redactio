import io
import json
from collections.abc import Callable
from typing import Any

import pytest

from redactio_sidecar import ipc
from redactio_sidecar.ipc import EngineError, run_loop

PAIR_ID = "11111111-1111-4111-8111-111111111111"
REVISION = "22222222-2222-4222-8222-222222222222"


def replies_for(
    raw: bytes, dispatch: Callable[[Any], dict[str, Any]] | None = None
) -> list[dict[str, Any]]:
    outgoing = io.BytesIO()
    run_loop(
        io.BytesIO(raw),
        outgoing,
        dispatch
        or (
            lambda request: {
                "id": request.id,
                "type": "ping_result",
                "payload": {"protocol_version": 1},
            }
        ),
    )
    return [json.loads(line) for line in outgoing.getvalue().splitlines()]


def process_request(request_id: str = "process") -> dict[str, Any]:
    return {
        "id": request_id,
        "type": "process_document",
        "payload": {
            "sync_pair_id": PAIR_ID,
            "doc_id": "doc-0001",
            "source_hash_sha256": "0" * 64,
            "processing_revision": REVISION,
            "redacted_at": "2026-09-19T10:00:00+02:00",
            "source_path": "synthetic.docx",
        },
    }


def review_request() -> dict[str, Any]:
    request = process_request("review")
    request["type"] = "render_review"
    request["payload"].update(
        {
            "detections": [
                {
                    "id": "detection-1",
                    "start": 0,
                    "end": 4,
                    "entity_type": "PERSON",
                    "confidence": 0.9,
                    "recognizer": "synthetic",
                    "origin": "automatic",
                }
            ],
            "decisions": {"dismissed_ids": [], "manual": []},
            "review_status": "pending",
            "reviewed_at": None,
            "acknowledged_warnings": [],
        }
    )
    return request


def encode(message: dict[str, Any]) -> bytes:
    return json.dumps(message, separators=(",", ":"), allow_nan=True).encode() + b"\n"


def test_invalid_payload_does_not_echo_private_input(capsys):
    incoming = io.BytesIO(
        b'{"id":"a","type":"ping","payload":{"secret":"CANARY_PATIENT"}}\n'
        b'{"id":"b","type":"ping","payload":{}}\n'
    )
    outgoing = io.BytesIO()

    def ping(request):
        return {
            "id": request.id,
            "type": "ping_result",
            "payload": {"protocol_version": 1},
        }

    run_loop(incoming, outgoing, ping)
    replies = [json.loads(line) for line in outgoing.getvalue().splitlines()]
    assert replies[0]["type"] == "error"
    assert replies[1]["id"] == "b"
    assert b"CANARY_PATIENT" not in outgoing.getvalue()
    assert "CANARY_PATIENT" not in capsys.readouterr().err


@pytest.mark.parametrize(
    ("raw", "expected_id"),
    [
        (b"\xff\n", ""),
        (b'{"id":\n', ""),
        (b'{"id":"unknown","type":"unknown","payload":{}}\n', "unknown"),
        (b'{"id":"version","type":"ping","payload":{"protocol_version":2}}\n', "version"),
        (b'{"id":"extra","type":"ping","payload":{},"secret":"private"}\n', "extra"),
    ],
)
def test_invalid_frames_return_only_safe_errors(raw, expected_id):
    assert replies_for(raw) == [
        {
            "id": expected_id,
            "type": "error",
            "payload": {"code": "invalid_request", "retryable": False},
        }
    ]


def test_nonfinite_confidence_is_an_invalid_request():
    request = review_request()
    request["payload"]["detections"][0]["confidence"] = float("nan")

    assert replies_for(encode(request))[0]["payload"] == {
        "code": "invalid_request",
        "retryable": False,
    }


def test_deeply_nested_json_returns_safe_error_and_loop_survives():
    nested = b"[" * 10_000 + b"]" * 10_000 + b"\n"
    ping = b'{"id":"after-recursion","type":"ping","payload":{}}\n'

    replies = replies_for(nested + ping)

    assert replies[0] == {
        "id": "",
        "type": "error",
        "payload": {"code": "invalid_request", "retryable": False},
    }
    assert replies[1]["id"] == "after-recursion"
    assert replies[1]["type"] == "ping_result"


def test_overlong_json_integer_returns_safe_error_and_loop_survives(capsys):
    overlong_integer = b"9" * 5_000 + b"\n"
    ping = b'{"id":"after-value-error","type":"ping","payload":{}}\n'

    replies = replies_for(overlong_integer + ping)

    assert replies == [
        {
            "id": "",
            "type": "error",
            "payload": {"code": "invalid_request", "retryable": False},
        },
        {
            "id": "after-value-error",
            "type": "ping_result",
            "payload": {"protocol_version": 1},
        },
    ]
    assert capsys.readouterr().err == ""


def test_over_limit_input_returns_error_and_stops(monkeypatch):
    monkeypatch.setattr(ipc, "MAX_MESSAGE_BYTES", 128)
    raw = b"x" * 129 + b'\n{"id":"later","type":"ping","payload":{}}\n'

    assert replies_for(raw) == [
        {
            "id": "",
            "type": "error",
            "payload": {"code": "message_too_large", "retryable": False},
        }
    ]


def test_over_limit_valid_output_becomes_safe_error(monkeypatch):
    monkeypatch.setattr(ipc, "MAX_MESSAGE_BYTES", 450)

    def oversized(request):
        return {
            "id": request.id,
            "type": "process_document_result",
            "payload": {
                **request.payload.model_dump(mode="json", exclude={"source_path"}),
                "markdown": "x" * 500,
                "body": "x" * 500,
                "original_text": "x" * 500,
                "detections": [],
                "redactions": [],
                "warnings": [],
                "body_was_empty": False,
                "review_status": "pending",
                "engine": {
                    "engine_version": "synthetic",
                    "model_name": "synthetic",
                    "model_version": "1",
                    "recognizers": [],
                    "extraction_version": "1",
                },
            },
        }

    assert replies_for(encode(process_request()), oversized)[0]["payload"] == {
        "code": "message_too_large",
        "retryable": False,
    }


def test_eof_terminates_without_output():
    assert replies_for(b"") == []


def test_engine_and_unexpected_errors_are_sanitized_and_loop_survives(capsys):
    calls = 0

    def fail_then_ping(request):
        nonlocal calls
        calls += 1
        if calls == 1:
            raise RuntimeError("CANARY_EXCEPTION")
        if calls == 2:
            raise EngineError("model_unavailable", retryable=True)
        return {
            "id": request.id,
            "type": "ping_result",
            "payload": {"protocol_version": 1},
        }

    raw = b"".join(
        encode({"id": request_id, "type": "ping", "payload": {}})
        for request_id in ("unexpected", "engine", "ok")
    )
    replies = replies_for(raw, fail_then_ping)

    assert [reply["payload"] for reply in replies[:2]] == [
        {"code": "internal_error", "retryable": False},
        {"code": "model_unavailable", "retryable": True},
    ]
    assert replies[2]["type"] == "ping_result"
    assert "CANARY_EXCEPTION" not in capsys.readouterr().err


def test_invalid_dispatch_response_is_sanitized():
    assert replies_for(
        b'{"id":"bad-output","type":"ping","payload":{}}\n',
        lambda _request: {"private": "CANARY_OUTPUT"},
    ) == [
        {
            "id": "bad-output",
            "type": "error",
            "payload": {"code": "internal_error", "retryable": False},
        }
    ]


@pytest.mark.parametrize(
    "response",
    [
        {"id": "wrong", "type": "ping_result", "payload": {"protocol_version": 1}},
        {
            "id": "ping",
            "type": "error",
            "payload": {"code": "returned_error", "retryable": False},
        },
    ],
)
def test_dispatch_must_return_the_correlated_success_type(response):
    assert replies_for(
        b'{"id":"ping","type":"ping","payload":{}}\n', lambda _request: response
    ) == [
        {
            "id": "ping",
            "type": "error",
            "payload": {"code": "internal_error", "retryable": False},
        }
    ]


def test_flushes_each_reply():
    class CountingBytesIO(io.BytesIO):
        flush_count = 0

        def flush(self):
            self.flush_count += 1
            super().flush()

    outgoing = CountingBytesIO()
    run_loop(
        io.BytesIO(
            b'{"id":"a","type":"ping","payload":{}}\n{"id":"b","type":"ping","payload":{}}\n'
        ),
        outgoing,
        lambda request: {
            "id": request.id,
            "type": "ping_result",
            "payload": {"protocol_version": 1},
        },
    )
    assert outgoing.flush_count == 2


def test_initial_dispatch_supports_ping_and_rejects_domain_commands():
    ping = ipc.REQUEST_ADAPTER.validate_python({"id": "ping", "type": "ping", "payload": {}})
    assert ipc.dispatch_request(ping) == {
        "id": "ping",
        "type": "ping_result",
        "payload": {"protocol_version": 1},
    }

    configure = ipc.REQUEST_ADAPTER.validate_python(
        {
            "id": "configure",
            "type": "configure",
            "payload": {
                "sync_pair_id": PAIR_ID,
                "processing_revision": REVISION,
                "config": {},
            },
        }
    )
    with pytest.raises(EngineError) as raised:
        ipc.dispatch_request(configure)
    assert raised.value.code == "unsupported_request"
