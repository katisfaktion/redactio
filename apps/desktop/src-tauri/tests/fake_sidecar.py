from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path
from uuid import UUID


def configure_result(request: dict) -> dict:
    return {
        "sync_pair_id": request["payload"]["sync_pair_id"],
        "processing_revision": request["payload"]["processing_revision"],
        "engine": {
            "engine_version": "synthetic-engine",
            "model_name": request["payload"]["config"]["model"],
            "model_version": "synthetic-model",
            "recognizers": ["synthetic-recognizer"],
            "extraction_version": "synthetic-extraction",
        },
    }


def main() -> None:
    mode = sys.argv[1]
    marker = Path(sys.argv[2]) if len(sys.argv) > 2 else None
    if mode == "never-read" and marker is not None:
        marker.write_text(str(os.getpid()), encoding="utf-8")
        time.sleep(5)
        return
    configured = None
    for line in sys.stdin:
        request = json.loads(line)
        if mode == "pid-hang" and marker is not None:
            marker.write_text(str(os.getpid()), encoding="utf-8")
            time.sleep(5)
            os._exit(0)
        if mode == "abort-steal" and marker is not None and not marker.exists():
            marker.write_text(str(os.getpid()), encoding="utf-8")
            next_request = json.loads(sys.stdin.readline())
            print(
                json.dumps(
                    {
                        "id": next_request["id"],
                        "type": f"{next_request['type']}_result",
                        "payload": next_request["payload"],
                    }
                ),
                flush=True,
            )
            time.sleep(5)
            continue
        if mode == "hang":
            time.sleep(60)
            continue
        if mode == "hang-count" and marker is not None:
            count = int(marker.read_text(encoding="utf-8")) if marker.exists() else 0
            marker.write_text(str(count + 1), encoding="utf-8")
            time.sleep(60)
            continue
        if mode == "exit-once" and marker is not None and not marker.exists():
            marker.write_text(request["id"], encoding="utf-8")
            os._exit(7)
        if mode == "exit-always":
            if marker is not None:
                count = (
                    int(marker.read_text(encoding="utf-8")) if marker.exists() else 0
                )
                marker.write_text(str(count + 1), encoding="utf-8")
            os._exit(7)
        if mode == "slow-exit-once" and marker is not None:
            time.sleep(0.08)
            if not marker.exists():
                marker.write_text("exited", encoding="utf-8")
                os._exit(7)
        if mode == "check-model-arg" and "--model-dir" not in sys.argv:
            print(
                json.dumps(
                    {
                        "id": request["id"],
                        "type": "error",
                        "payload": {"code": "missing_model_arg", "retryable": False},
                    }
                ),
                flush=True,
            )
            continue
        if mode == "clean-python-env" and (
            "PYTHONPATH" in os.environ or "PYTHONHOME" in os.environ
        ):
            print(
                json.dumps(
                    {
                        "id": request["id"],
                        "type": "error",
                        "payload": {"code": "unsafe_python_env", "retryable": False},
                    }
                ),
                flush=True,
            )
            continue
        if mode == "exit-once-configured" and marker is not None:
            if request["type"] == "configure":
                configured = request["payload"]
                with marker.with_suffix(".configs").open("a", encoding="utf-8") as log:
                    log.write(json.dumps(configured, sort_keys=True) + "\n")
            elif configured is None:
                print(
                    json.dumps(
                        {
                            "id": request["id"],
                            "type": "error",
                            "payload": {
                                "code": "configuration_mismatch",
                                "retryable": False,
                            },
                        }
                    ),
                    flush=True,
                )
                continue
            elif not marker.exists():
                marker.write_text(request["id"], encoding="utf-8")
                os._exit(7)
        if (
            mode
            in {
                "timeout-then-require-config",
                "invalid-configure-then-exit",
                "invalid-configure-replay",
                "replay-exit-once",
                "replay-exit-twice",
                "replay-hang",
            }
            and marker is not None
        ):
            config_log = marker.with_suffix(".configs")
            if request["type"] == "configure":
                configured = request["payload"]
                with config_log.open("a", encoding="utf-8") as log:
                    log.write(json.dumps(configured, sort_keys=True) + "\n")
                configure_count = len(
                    config_log.read_text(encoding="utf-8").splitlines()
                )
                if mode == "replay-exit-once" and configure_count == 2:
                    os._exit(7)
                if mode == "replay-exit-twice" and configure_count in {2, 3}:
                    os._exit(7)
                if mode == "replay-hang" and configure_count == 2:
                    marker.write_text("replaying", encoding="utf-8")
                    time.sleep(5)
                    continue
            elif request["type"] == "process_document":
                if mode == "timeout-then-require-config" and not marker.exists():
                    marker.write_text("timed", encoding="utf-8")
                    time.sleep(5)
                    continue
                if (
                    mode in {"invalid-configure-then-exit", "invalid-configure-replay"}
                    and not marker.exists()
                ):
                    marker.write_text("exited", encoding="utf-8")
                    os._exit(7)
                if configured is None:
                    print(
                        json.dumps(
                            {
                                "id": request["id"],
                                "type": "error",
                                "payload": {
                                    "code": "configuration_mismatch",
                                    "retryable": False,
                                },
                            }
                        ),
                        flush=True,
                    )
                    continue
        payload = request["payload"]
        if mode.startswith("batch") and request["type"] == "configure":
            configured = request["payload"]
            if mode == "batch-slow-init":
                time.sleep(0.3)
        if mode.startswith("batch") and request["type"] in {
            "process_document",
            "render_review",
        }:
            source = Path(payload["source_path"]).read_bytes()
            if source == b"invalid":
                print(
                    json.dumps(
                        {
                            "id": request["id"],
                            "type": "error",
                            "payload": {"code": "invalid_docx", "retryable": False},
                        }
                    ),
                    flush=True,
                )
                continue
            if source == b"change":
                Path(payload["source_path"]).write_bytes(b"changed during processing")
            warnings = ["unsupported_images"] if source == b"warning" else []
            payload = {
                key: payload[key]
                for key in [
                    "sync_pair_id",
                    "doc_id",
                    "source_hash_sha256",
                    "processing_revision",
                    "redacted_at",
                ]
            } | {
                "markdown": f"{payload['sync_pair_id']} {payload['doc_id']} {payload['redacted_at']}\nSynthetic output",
                "body": "Synthetic output",
                "original_text": "CANARY_PRIVATE_ORIGINAL",
                "detections": [],
                "redactions": [],
                "warnings": warnings,
                "body_was_empty": False,
                "review_status": "needs-rework" if warnings else "pending",
                "engine": configure_result({"payload": configured})["engine"],
            }
        if request["type"] == "configure":
            payload = configure_result(request)
            if mode == "invalid-configure-then-exit":
                payload.pop("engine")
            if mode == "invalid-configure-replay" and marker is not None:
                configure_count = len(
                    marker.with_suffix(".configs")
                    .read_text(encoding="utf-8")
                    .splitlines()
                )
                if configure_count > 1:
                    payload.pop("engine")
        if mode == "exit-once" and marker is not None:
            payload = {"retry_id": request["id"]}
        if mode == "wrong-pair":
            payload = dict(payload)
            payload["sync_pair_id"] = str(UUID(int=2))
        response = {
            "id": request["id"],
            "type": f"{request['type']}_result",
            "payload": payload,
        }
        if mode == "safe-error":
            response["type"] = "error"
            response["payload"] = {"code": "invalid_configuration", "retryable": False}
        if mode == "unsafe-error":
            response["type"] = "error"
            response["payload"] = {"code": "CANARY PRIVATE", "retryable": False}
        if mode == "oversized":
            response["payload"] = {"padding": "x" * (64 * 1024 * 1024)}
        if mode == "extra-envelope":
            response["extra"] = True
        if mode == "wrong-type":
            response["type"] = "configure_result"
        if mode == "deeply-nested":
            sys.stdout.write(
                '{"id":'
                + json.dumps(request["id"])
                + ',"type":"ping_result","payload":'
                + "[" * 200
                + "null"
                + "]" * 200
                + "}\n"
            )
            sys.stdout.flush()
            continue
        if mode == "stderr-canary":
            sys.stderr.write("CANARY_PRIVATE_STDERR" * 65536)
            sys.stderr.flush()
        if mode == "wrong-id":
            print(json.dumps({**response, "id": str(UUID(int=4))}), flush=True)
        if mode == "exit-once-configured" and request["type"] != "configure":
            response["payload"] = {
                "sync_pair_id": request["payload"]["sync_pair_id"],
                "processing_revision": request["payload"]["processing_revision"],
                "configured_model": configured["config"]["model"],
            }
        if (
            mode
            in {
                "timeout-then-require-config",
                "invalid-configure-then-exit",
                "invalid-configure-replay",
                "replay-exit-once",
                "replay-exit-twice",
                "replay-hang",
            }
            and request["type"] == "process_document"
            and configured is not None
        ):
            response["payload"] = {
                "sync_pair_id": request["payload"]["sync_pair_id"],
                "processing_revision": request["payload"]["processing_revision"],
                "configured_model": configured["config"]["model"],
            }
        print(json.dumps(response), flush=True)
    if mode == "eof-marker" and marker is not None:
        marker.write_text("eof", encoding="utf-8")


if __name__ == "__main__":
    main()
