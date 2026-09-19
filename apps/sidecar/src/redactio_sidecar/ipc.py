from __future__ import annotations

import json
from collections.abc import Callable
from typing import Any, BinaryIO

from pydantic import TypeAdapter, ValidationError

from .schemas import PingRequest, Request, Response

MAX_MESSAGE_BYTES = 64 * 1024 * 1024
REQUEST_ADAPTER: TypeAdapter[Request] = TypeAdapter(Request)
RESPONSE_ADAPTER: TypeAdapter[Response] = TypeAdapter(Response)


class EngineError(Exception):
    def __init__(self, code: str, retryable: bool = False) -> None:
        super().__init__(code)
        self.code = code
        self.retryable = retryable


def read_frame(stream: BinaryIO) -> bytes:
    line = stream.readline(MAX_MESSAGE_BYTES + 1)
    if len(line) > MAX_MESSAGE_BYTES:
        raise EngineError("message_too_large")
    return line


def _error_response(request_id: str, error: EngineError) -> dict[str, Any]:
    return {
        "id": request_id,
        "type": "error",
        "payload": {"code": error.code, "retryable": error.retryable},
    }


def _request_id(value: object) -> str:
    if not isinstance(value, dict):
        return ""
    request_id = value.get("id")
    if isinstance(request_id, str) and 1 <= len(request_id) <= 128:
        return request_id
    return ""


def _serialize(
    response: dict[str, Any], expected_id: str | None = None, expected_type: str | None = None
) -> bytes:
    validated = RESPONSE_ADAPTER.validate_python(response)
    if expected_id is not None and (validated.id != expected_id or validated.type != expected_type):
        raise EngineError("internal_error")
    encoded = RESPONSE_ADAPTER.dump_json(validated)
    if len(encoded) > MAX_MESSAGE_BYTES:
        raise EngineError("message_too_large")
    return encoded


def _write(
    stdout: BinaryIO,
    response: dict[str, Any],
    expected_id: str | None = None,
    expected_type: str | None = None,
) -> None:
    stdout.write(_serialize(response, expected_id, expected_type) + b"\n")
    stdout.flush()


def _write_error(stdout: BinaryIO, request_id: str, error: EngineError) -> None:
    try:
        _write(stdout, _error_response(request_id, error))
    except (EngineError, ValidationError):
        _write(stdout, _error_response("", EngineError("internal_error")))


def run_loop(
    stdin: BinaryIO,
    stdout: BinaryIO,
    dispatch: Callable[[Request], dict[str, Any]],
) -> None:
    while True:
        try:
            frame = read_frame(stdin)
        except EngineError as error:
            _write_error(stdout, "", error)
            return
        if not frame:
            return

        request_id = ""
        try:
            decoded = frame.decode("utf-8", errors="strict")
            raw_request = json.loads(decoded)
            request_id = _request_id(raw_request)
            request = REQUEST_ADAPTER.validate_python(raw_request)
        except (UnicodeDecodeError, json.JSONDecodeError, RecursionError, ValidationError):
            _write_error(stdout, request_id, EngineError("invalid_request"))
            continue

        try:
            _write(stdout, dispatch(request), request.id, f"{request.type}_result")
        except EngineError as error:
            _write_error(stdout, request.id, error)
        except Exception:
            _write_error(stdout, request.id, EngineError("internal_error"))


def dispatch_request(request: Request) -> dict[str, Any]:
    if isinstance(request, PingRequest):
        return {
            "id": request.id,
            "type": "ping_result",
            "payload": {"protocol_version": 1},
        }
    raise EngineError("unsupported_request")


def main() -> None:
    import sys

    run_loop(sys.stdin.buffer, sys.stdout.buffer, dispatch_request)
