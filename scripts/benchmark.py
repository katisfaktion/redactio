"""Benchmark the packaged engine, never the GUI or human review. Stdlib Python 3.11+."""

import argparse
import ctypes
import hashlib
import json
import math
import os
import platform
import queue
import re
import stat
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from datetime import UTC, datetime
from pathlib import Path
from uuid import uuid4

MAX_MESSAGE_BYTES = 64 * 1024 * 1024
INITIALIZATION_SECONDS = 180
DOCUMENT_SECONDS = 120
ENTITIES = [
    "EMAIL_ADDRESS",
    "PHONE_NUMBER",
    "IBAN_CODE",
    "IP_ADDRESS",
    "URL",
    "DATE_TIME",
]


class BenchmarkError(Exception):
    """Only fixed, content-free diagnostics may cross the CLI boundary."""


def require(condition, code="invalid_response"):
    if not condition:
        raise BenchmarkError(code)


def machine_info():
    if sys.platform == "win32":
        import winreg

        class MemoryStatus(ctypes.Structure):
            _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
                (name, ctypes.c_ulonglong)
                for name in (
                    "total",
                    "available",
                    "page_total",
                    "page_available",
                    "virtual_total",
                    "virtual_available",
                    "extended",
                )
            ]

        memory = MemoryStatus()
        memory.length = ctypes.sizeof(memory)
        require(
            ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(memory)), "memory_unavailable"
        )
        ram = memory.total
        with winreg.OpenKey(
            winreg.HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"
        ) as key:
            cpu = winreg.QueryValueEx(key, "ProcessorNameString")[0].strip()
    elif sys.platform == "linux":
        ram = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
        cpu = next(
            line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")
        )
    else:
        raise BenchmarkError("unsupported_os")
    return {
        "os": platform.system(),
        "os_release": platform.release(),
        "os_version": platform.version(),
        "architecture": platform.machine(),
        "cpu": cpu,
        "logical_cpus": os.cpu_count(),
        "ram_bytes": ram,
    }


def windows_peak(process):
    class Counters(ctypes.Structure):
        _fields_ = [("cb", ctypes.c_ulong), ("faults", ctypes.c_ulong)] + [
            (name, ctypes.c_size_t)
            for name in (
                "peak",
                "working",
                "peak_paged",
                "paged",
                "peak_nonpaged",
                "nonpaged",
                "pagefile",
                "peak_pagefile",
            )
        ]

    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    require(
        ctypes.windll.psapi.GetProcessMemoryInfo(
            ctypes.c_void_p(int(process._handle)), ctypes.byref(counters), counters.cb
        ),
        "memory_unavailable",
    )
    return counters.peak


class Child:
    """One bounded exchange at a time; errors always kill/reap the owned process."""

    def __init__(self, command, env, cwd):
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=env,
            cwd=cwd,
        )
        self.worker = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        if self.process.returncode is None:
            self.process.kill()
            self.process.wait(timeout=5)
        if self.worker:
            self.worker.join(timeout=5)
        self.process.stdin.close()
        self.process.stdout.close()

    def request(self, kind, payload, deadline, limit=MAX_MESSAGE_BYTES):
        request_id = str(uuid4())
        encoded = json.dumps({"id": request_id, "type": kind, "payload": payload}).encode() + b"\n"
        require(len(encoded) <= limit, "request_too_large")
        result = queue.Queue(maxsize=1)

        def exchange():
            try:
                self.process.stdin.write(encoded)
                self.process.stdin.flush()
                line = self.process.stdout.readline(limit + 1)
                require(len(line) <= limit and line.endswith(b"\n"), "invalid_frame")
                result.put(json.loads(line.decode("utf-8")))
            except Exception:
                result.put(None)

        self.worker = threading.Thread(target=exchange, daemon=True)
        self.worker.start()
        try:
            reply = result.get(timeout=max(0, deadline - time.perf_counter()))
        except queue.Empty:
            raise BenchmarkError("sidecar_deadline") from None
        self.worker.join()
        require(isinstance(reply, dict) and set(reply) == {"id", "type", "payload"})
        require(reply["id"] == request_id and isinstance(reply["payload"], dict))
        require(reply["type"] in (kind + "_result", "error"))
        if reply["type"] == "error":
            error = reply["payload"]
            require(
                set(error) == {"code", "retryable"}
                and isinstance(error["code"], str)
                and re.fullmatch(r"[a-z][a-z0-9_]{0,127}", error["code"])
                and type(error["retryable"]) is bool
            )
            require(kind == "process_document", "initialization_failed")
            return None
        return reply["payload"]

    def finish(self):
        self.process.stdin.close()
        if sys.platform == "win32":
            self.process.wait(timeout=5)
            peak = windows_peak(self.process)
        else:
            # wait4 reports this child's lifetime maximum, not RUSAGE_CHILDREN's
            # cumulative maximum across unrelated earlier child processes.
            deadline = time.perf_counter() + 5
            while True:
                pid, status, usage = os.wait4(self.process.pid, os.WNOHANG)
                if pid:
                    self.process.returncode = os.waitstatus_to_exitcode(status)
                    peak = usage.ru_maxrss * 1024
                    break
                require(time.perf_counter() < deadline, "shutdown_deadline")
                time.sleep(0.01)
        require(self.process.returncode == 0, "sidecar_failed")
        require(peak > 0, "memory_unavailable")
        return peak


def distribution(values):
    return {
        "count": len(values),
        "min": min(values) if values else None,
        "max": max(values) if values else None,
        "total": sum(values),
        "median": statistics.median(values) if values else None,
    }


def engine_identity(value):
    require(
        isinstance(value, dict)
        and set(value)
        == {"engine_version", "model_name", "model_version", "extraction_version", "recognizers"}
    )
    fields = ("engine_version", "model_name", "model_version", "extraction_version")
    require(
        all(
            isinstance(value[key], str) and re.fullmatch(r"[A-Za-z0-9_.+ -]{1,128}", value[key])
            for key in fields
        )
    )
    require(
        isinstance(value["recognizers"], list)
        and all(isinstance(item, str) and 0 < len(item) <= 128 for item in value["recognizers"])
    )
    return {key: value[key] for key in fields}


def validate_process(result, request, engine):
    require(
        set(result)
        == set(request) - {"source_path"}
        | {
            "markdown",
            "body",
            "original_text",
            "detections",
            "redactions",
            "warnings",
            "body_was_empty",
            "review_status",
            "engine",
        }
    )
    require(all(result[key] == value for key, value in request.items() if key != "source_path"))
    require(engine_identity(result["engine"]) == engine)
    require(all(isinstance(result[key], str) for key in ("markdown", "body", "original_text")))
    require(
        len(result["original_text"]) <= 1_000_000
        and type(result["body_was_empty"]) is bool
        and result["review_status"] == "pending"
    )
    require(all(isinstance(result[key], list) for key in ("detections", "redactions", "warnings")))
    require(
        all(
            isinstance(code, str) and re.fullmatch(r"[a-z][a-z0-9_]{0,127}", code)
            for code in result["warnings"]
        )
    )
    for entry in result["detections"]:
        require(
            isinstance(entry, dict)
            and set(entry)
            == {"id", "start", "end", "entity_type", "confidence", "recognizer", "origin"}
        )
        require(
            type(entry["start"]) is int
            and type(entry["end"]) is int
            and 0 <= entry["start"] < entry["end"] <= len(result["original_text"])
        )
    for entry in result["redactions"]:
        require(
            isinstance(entry, dict)
            and set(entry)
            == {
                "start_offset",
                "end_offset",
                "entity_type",
                "placeholder",
                "confidence",
                "recognizer",
                "origin",
            }
        )
        require(
            type(entry["start_offset"]) is int
            and type(entry["end_offset"]) is int
            and 0 <= entry["start_offset"] < entry["end_offset"] <= len(result["body"])
        )
        require(result["body"][entry["start_offset"] : entry["end_offset"]] == entry["placeholder"])


def validate_metrics(result):
    assert result["documents"] > 0
    assert math.isfinite(result["cold_start_seconds"]) and result["cold_start_seconds"] >= 0
    assert math.isfinite(result["total_seconds"])
    assert result["total_seconds"] >= result["cold_start_seconds"]
    assert result["processed"] >= 0 and result["failed"] >= 0
    assert result["processed"] + result["failed"] == result["documents"]
    assert result["peak_memory_bytes"] > 0
    assert "filenames" not in result and "text" not in result
    assert result["input_bytes"]["count"] == result["documents"]
    assert result["characters"]["count"] == result["processed"]
    assert result["failed"] == 0 or result["documents_per_minute"] is None


def run_benchmark(command, model_root, corpus):
    machine = machine_info()
    require(corpus.is_dir(), "invalid_corpus")

    def scan_failed(_):
        raise BenchmarkError("incomplete_corpus_scan")

    files = []
    for root, directories, names in os.walk(corpus, onerror=scan_failed):
        for name in directories + names:
            path = Path(root) / name
            attributes = path.lstat()
            require(
                not path.is_symlink()
                and not getattr(attributes, "st_file_attributes", 0)
                & stat.FILE_ATTRIBUTE_REPARSE_POINT,
                "redirected_corpus_entry",
            )
            if path.suffix.lower() == ".docx":
                require(stat.S_ISREG(attributes.st_mode), "invalid_corpus")
                files.append(path)
    files.sort()
    require(bool(files), "empty_corpus")
    sizes = [path.stat().st_size for path in files]
    require(all(size <= MAX_MESSAGE_BYTES for size in sizes), "input_too_large")
    manifest = json.loads((model_root / "manifest.json").read_text(encoding="utf-8"))
    require(
        isinstance(manifest, dict)
        and isinstance(manifest.get("models"), list)
        and len(manifest["models"]) == 1,
        "invalid_model_manifest",
    )
    model = manifest["models"][0]
    require(
        model.get("name") == "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
        and model.get("path") == "biomedbert-de",
        "invalid_model_manifest",
    )
    metadata = json.loads((model_root / "biomedbert-de/config.json").read_text(encoding="utf-8"))
    labels = metadata.get("id2label")
    require(isinstance(labels, dict) and bool(labels), "invalid_model_manifest")
    require(all(isinstance(label, str) for label in labels.values()), "invalid_model_manifest")
    model_entities = sorted({re.sub(r"^[BI]-", "", label) for label in labels.values()} - {"O"})
    require(
        bool(model_entities)
        and all(re.fullmatch(r"[A-Z][A-Z0-9_]{0,63}", label) for label in model_entities),
        "invalid_model_manifest",
    )
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.upper().startswith(("PYTHON", "REDACTIO_"))
    }
    env["PYTHONUTF8"] = "1"
    if sys.platform == "win32":
        env["PATH"] = os.pathsep.join(
            (str(Path(env["SYSTEMROOT"]) / "System32"), env["SYSTEMROOT"])
        )
    else:
        env["PATH"] = "/usr/bin:/bin"
    pair = str(uuid4())
    revision = str(uuid4())
    characters = []
    failures = warnings = empty = detections = redactions = 0
    with tempfile.TemporaryDirectory(prefix="redactio-benchmark-") as temporary:
        start = time.perf_counter()
        with Child(command, env, temporary) as child:
            deadline = start + INITIALIZATION_SECONDS
            require(child.request("ping", {}, deadline) == {"protocol_version": 1})
            configured = child.request(
                "configure",
                {
                    "sync_pair_id": pair,
                    "processing_revision": revision,
                    "config": {
                        "model": model["name"],
                        "model_entities": model_entities,
                        "enabled_entities": ENTITIES,
                        "custom_rules": [],
                        "include_positions": True,
                    },
                },
                deadline,
            )
            require(set(configured) == {"sync_pair_id", "processing_revision", "engine"})
            require(
                configured["sync_pair_id"] == pair and configured["processing_revision"] == revision
            )
            engine = engine_identity(configured["engine"])
            require(
                engine["model_name"] == model["name"]
                and engine["model_version"] == model["version"]
            )
            cold_start = time.perf_counter() - start
            for number, path in enumerate(files, 1):
                with path.open("rb") as stream:
                    digest = hashlib.file_digest(stream, "sha256").hexdigest()
                request = {
                    "sync_pair_id": pair,
                    "processing_revision": revision,
                    "doc_id": f"doc-{number:04d}",
                    "source_path": str(path.resolve()),
                    "source_hash_sha256": digest,
                    "redacted_at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
                }
                result = child.request(
                    "process_document", request, time.perf_counter() + DOCUMENT_SECONDS
                )
                if result is None:
                    failures += 1
                    continue
                validate_process(result, request, engine)
                characters.append(len(result["original_text"]))
                warnings += bool(result["warnings"])
                empty += result["body_was_empty"]
                detections += len(result["detections"])
                redactions += len(result["redactions"])
            peak = child.finish()
        total = time.perf_counter() - start
    metrics = {
        "schema_version": 1,
        "scope": "packaged_engine_only",
        "recorded_at": datetime.now(UTC).isoformat(),
        "documents": len(files),
        "processed": len(characters),
        "failed": failures,
        "warning_documents": warnings,
        "empty_documents": empty,
        "detections": detections,
        "redactions": redactions,
        "input_bytes": distribution(sizes),
        "characters": distribution(characters),
        "cold_start_seconds": cold_start,
        "total_seconds": total,
        "documents_per_minute": len(files) * 60 / total if not failures else None,
        "peak_memory_bytes": peak,
        "peak_memory_scope": "sidecar_process_lifetime",
        "peak_memory_method": "PeakWorkingSetSize"
        if sys.platform == "win32"
        else "wait4_ru_maxrss",
        "machine": machine,
        "engine": engine,
    }
    validate_metrics(metrics)
    return metrics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        require(not args.output.exists() and not args.output.is_symlink(), "output_exists")
        package = args.package.resolve(strict=True)
        corpus = args.corpus.resolve(strict=True)
        executable = package / "sidecar" / "redactio-sidecar.exe"
        require(executable.is_file(), "invalid_package")
        result = run_benchmark(
            [str(executable), "--model-dir", str(package / "models")], package / "models", corpus
        )
        with args.output.open("x", encoding="utf-8") as output:
            output.write(json.dumps(result, indent=2, allow_nan=False) + "\n")
        print(f"benchmark_complete processed={result['processed']} failed={result['failed']}")
        return 1 if result["failed"] else 0
    except Exception:
        # Never echo source paths, malformed reply bodies, or child stderr.
        print(
            "benchmark_failed: check inputs, package, deadlines and a new output path",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    sys.exit(main())
