"""Explicit, isolated model management. Document IPC never calls this module."""

from __future__ import annotations

import hashlib
import http.client
import ipaddress
import json
import os
import queue
import re
import shutil
import socket
import ssl
import stat
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Any, cast
from urllib.parse import urljoin, urlsplit

import certifi
from pydantic import TypeAdapter

from .ipc import EngineError
from .model_store import (
    Artifact,
    ModelDescriptor,
    ModelJob,
    ModelRecord,
    ModelSource,
    Repository,
    UpstreamHash,
    _safe_path,
    catalog_models,
    directory_name,
    processing_window,
    read_registry,
    selection_name,
    validate_local_model,
    write_registry,
)

JSON_LIMIT = 2 * 1024 * 1024
TOKENIZER_LIMIT = 64 * 1024 * 1024
WEIGHT_LIMIT = 8 * 1024**3
TIMEOUT = 30
CHUNK = 1024 * 1024
HOSTS = frozenset(
    {
        "huggingface.co",
        "cdn-lfs.huggingface.co",
        "cdn-lfs.hf.co",
        "cdn-lfs-us-1.hf.co",
        "cdn-lfs-eu-1.hf.co",
        "cas-bridge.xethub.hf.co",
    }
)
FILES = frozenset(
    {
        "config.json",
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "added_tokens.json",
        "vocab.txt",
        "vocab.json",
        "merges.txt",
        "spm.model",
        "sentencepiece.bpe.model",
        "model.safetensors",
        "README.md",
    }
)
REQUIRED = frozenset(
    {"config.json", "tokenizer.json", "tokenizer_config.json", "model.safetensors"}
)


def parse_repository_url(url: str) -> str:
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or any(ord(c) < 33 or ord(c) > 126 for c in url)
        or parsed.netloc != "huggingface.co"
        or "?" in url
        or "#" in url
        or "%" in url
        or not url.startswith("https://huggingface.co/")
    ):
        raise ValueError("invalid repository URL")
    return TypeAdapter(Repository).validate_python(parsed.path[1:].removesuffix("/"), strict=True)


def object_digest(data: bytes, algorithm: str) -> str:
    digest = _hasher(algorithm, len(data))
    digest.update(data)
    return str(digest.hexdigest())


def _hasher(algorithm: str, size: int) -> Any:
    if algorithm == "sha256":
        return hashlib.sha256()
    if algorithm == "git-sha1":
        return hashlib.sha1(f"blob {size}\0".encode())
    raise ValueError("unsupported object hash")


def _checked_url(url: str) -> Any:
    try:
        parsed = urlsplit(url)
        if (
            parsed.scheme != "https"
            or parsed.netloc not in HOSTS
            or parsed.fragment
            or len(url) > 16384
            or any(ord(c) < 33 or ord(c) > 126 for c in url)
            or "\\" in url
        ):
            raise ValueError("unsafe URL")
        return parsed
    except ValueError as error:
        raise EngineError("model_network_unsafe") from error


def _public_ip(address: str) -> bool:
    ip = ipaddress.ip_address(address)
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped:
        ip = ip.ipv4_mapped
    return ip.is_global and not (ip.is_multicast or ip.is_unspecified or ip.is_reserved)


class _PublicHTTPSConnection(http.client.HTTPSConnection):
    _context: ssl.SSLContext

    def connect(self) -> None:
        # Connect the checked numeric address itself: no second DNS lookup or proxy routing.
        deadline = time.monotonic() + TIMEOUT
        resolved: queue.Queue[Any] = queue.Queue(maxsize=1)

        def resolve() -> None:
            try:
                resolved.put(socket.getaddrinfo(self.host, 443, type=socket.SOCK_STREAM))
            except Exception as error:
                resolved.put(error)

        # libc DNS has no per-call timeout; a daemon cannot keep a failed worker alive.
        threading.Thread(target=resolve, daemon=True).start()
        try:
            addresses = resolved.get(timeout=TIMEOUT)
        except queue.Empty as error:
            raise TimeoutError("DNS timeout") from error
        if isinstance(addresses, Exception):
            raise addresses
        if not addresses or any(not _public_ip(str(item[4][0])) for item in addresses):
            raise EngineError("model_network_unsafe")
        last: OSError | None = None
        for family, kind, proto, _, address in addresses:
            raw = socket.socket(family, kind, proto)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raw.close()
                raise TimeoutError("connection timeout")
            raw.settimeout(remaining)
            try:
                raw.connect(address)
                if raw.getpeername()[0] != address[0] or not _public_ip(raw.getpeername()[0]):
                    raise EngineError("model_network_unsafe")
                self.sock = self._context.wrap_socket(raw, server_hostname=self.host)
                self.sock.settimeout(TIMEOUT)
                return
            except BaseException as error:
                raw.close()
                if not isinstance(error, OSError) or isinstance(error, ssl.SSLError):
                    raise
                last = error
        raise last or OSError("connection failed")


@contextmanager
def _request(url: str) -> Iterator[http.client.HTTPResponse]:
    parsed = _checked_url(url)
    connection = _PublicHTTPSConnection(
        parsed.hostname, timeout=TIMEOUT, context=ssl.create_default_context(cafile=certifi.where())
    )
    try:
        target = parsed.path or "/"
        if parsed.query:
            target += "?" + parsed.query
        connection.request(
            "GET",
            target,
            headers={"Accept-Encoding": "identity", "User-Agent": "Redactio-model-manager"},
        )
        with connection.getresponse() as response:
            yield response
    finally:
        connection.close()


@contextmanager
def open_checked(url: str) -> Iterator[http.client.HTTPResponse]:
    try:
        for redirects in range(6):
            _checked_url(url)
            with _request(url) as response:
                if response.status in (301, 302, 303, 307, 308):
                    location = response.getheader("Location")
                    if not location or redirects == 5:
                        raise EngineError("model_network_unsafe")
                    url = urljoin(url, location)
                    continue
                if response.status in (401, 403):
                    raise EngineError("model_access_denied")
                if response.status == 404:
                    raise EngineError("model_unavailable")
                if response.status != 200:
                    raise EngineError("model_network_failed", retryable=True)
                if response.getheader("Content-Encoding", "identity") != "identity":
                    raise EngineError("model_metadata_invalid")
                yield response
                return
    except TimeoutError as error:
        raise EngineError("model_network_timeout", retryable=True) from error
    except (OSError, http.client.HTTPException) as error:
        raise EngineError("model_network_failed", retryable=True) from error


def _chunks(
    response: http.client.HTTPResponse, maximum: int, seconds: int = TIMEOUT
) -> Iterator[bytes]:
    deadline, count = time.monotonic() + seconds, 0
    while True:
        if time.monotonic() > deadline:
            raise EngineError("model_network_timeout", retryable=True)
        chunk = response.read1(min(CHUNK, maximum - count + 1))
        if time.monotonic() > deadline:
            raise EngineError("model_network_timeout", retryable=True)
        if not chunk:
            break
        count += len(chunk)
        if count > maximum:
            raise EngineError("model_size_mismatch")
        yield chunk


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = dict(pairs)
    if len(result) != len(pairs):
        raise ValueError("duplicate JSON field")
    return result


def _decode_json(data: bytes) -> dict[str, Any]:
    try:
        result = json.loads(data, object_pairs_hook=_unique_object)
        if not isinstance(result, dict):
            raise ValueError("not an object")
        return result
    except (ValueError, RecursionError) as error:
        raise EngineError("model_metadata_invalid") from error


def _json_url(url: str, artifact: Artifact | None = None) -> dict[str, Any]:
    with open_checked(url) as response:
        data = b"".join(_chunks(response, JSON_LIMIT))
    if artifact is not None and (
        len(data) != artifact.size
        or object_digest(data, artifact.upstream_hash.algorithm) != artifact.upstream_hash.value
    ):
        raise EngineError("model_hash_mismatch")
    return _decode_json(data)


def _file_limit(filename: str) -> int:
    if filename == "model.safetensors":
        return WEIGHT_LIMIT
    return (
        TOKENIZER_LIMIT
        if filename
        in {"tokenizer.json", "vocab.txt", "vocab.json", "spm.model", "sentencepiece.bpe.model"}
        else JSON_LIMIT
    )


def _check_files(files: list[Artifact]) -> None:
    if not REQUIRED <= {file.filename for file in files}:
        raise EngineError("model_artifacts_missing")
    if any(
        file.filename not in FILES or not 0 < file.size <= _file_limit(file.filename)
        for file in files
    ):
        raise EngineError("model_artifacts_unsafe")


def _config(
    config: dict[str, Any], tokenizer: dict[str, Any]
) -> tuple[str, str, list[str], int, int]:
    try:
        if config.get("auto_map") or tokenizer.get("auto_map"):
            raise EngineError("model_remote_code_unsupported")
        family = config["model_type"]
        architecture = {
            "bert": "BertForTokenClassification",
            "deberta-v2": "DebertaV2ForTokenClassification",
        }[family]
        if config["architectures"] != [architecture]:
            raise ValueError("architecture")
        labels = config["id2label"]
        if (
            not isinstance(labels, dict)
            or set(labels) != {str(i) for i in range(len(labels))}
            or not labels
            or len(labels) > 4096
        ):
            raise ValueError("labels")
        if any(
            not isinstance(label, str)
            or re.fullmatch(r"LABEL_\d+", label)
            or not re.fullmatch(r"(?:[BI]-)?[A-Z][A-Z0-9_]{0,63}", label)
            for label in labels.values()
        ):
            raise EngineError("model_labels_invalid")
        if len(set(labels.values())) != len(labels):
            raise EngineError("model_labels_invalid")
        if "label2id" in config and (
            not isinstance(config["label2id"], dict)
            or any(type(index) is not int for index in config["label2id"].values())
            or config["label2id"] != {label: int(index) for index, label in labels.items()}
        ):
            raise EngineError("model_labels_invalid")
        if "num_labels" in config and config["num_labels"] != len(labels):
            raise EngineError("model_labels_invalid")
        entities = sorted(
            {label[2:] if label.startswith(("B-", "I-")) else label for label in labels.values()}
            - {"O"}
        )
        if not entities:
            raise EngineError("model_labels_invalid")
        window = processing_window(
            config["max_position_embeddings"], tokenizer.get("model_max_length"), 0
        )
        return family, architecture, entities, window.tokens, window.stride
    except (KeyError, TypeError, ValueError) as error:
        raise EngineError("model_incompatible") from error


def _resolve(repository: str, revision: str, filename: str) -> str:
    return f"https://huggingface.co/{repository}/resolve/{revision}/{filename}"


def preflight(source: ModelSource, root: Path) -> ModelDescriptor:
    if source.kind == "catalog":
        return next(entry.descriptor for entry in catalog_models() if entry.key == source.key)
    if source.kind == "receipt":
        for record in read_registry(root).models:
            if record.descriptor.name == source.name:
                return record.descriptor
        raise EngineError("model_not_found")
    repository = parse_repository_url(source.url)
    endpoint = f"https://huggingface.co/api/models/{repository}"
    info = _json_url(endpoint)
    revision = info.get("sha")
    if info.get("id") != repository:
        raise EngineError("model_source_mismatch")
    if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise EngineError("model_metadata_invalid")
    if info.get("gated") not in (False, None) or info.get("private") is not False:
        raise EngineError("model_access_denied")
    info = _json_url(f"{endpoint}/revision/{revision}?blobs=true")
    if info.get("id") != repository or info.get("sha") != revision:
        raise EngineError("model_source_mismatch")
    if info.get("gated") not in (False, None) or info.get("private") is not False:
        raise EngineError("model_access_denied")
    try:
        artifacts = []
        for sibling in info["siblings"]:
            filename = sibling["rfilename"]
            if filename not in FILES or filename == "README.md":
                continue
            lfs = sibling.get("lfs")
            if lfs is not None:
                if type(lfs["size"]) is not int or lfs["size"] != sibling["size"]:
                    raise ValueError("ambiguous size")
                algorithm, value = "sha256", lfs["sha256"]
            else:
                algorithm, value = "git-sha1", sibling["blobId"]
            artifacts.append(
                Artifact(
                    filename=filename,
                    size=sibling["size"],
                    upstream_hash=UpstreamHash.model_validate(
                        {"algorithm": algorithm, "value": value}
                    ),
                    sha256=None,
                )
            )
        _check_files(artifacts)
        by_name = {file.filename: file for file in artifacts}
        config = _json_url(_resolve(repository, revision, "config.json"), by_name["config.json"])
        tokenizer = _json_url(
            _resolve(repository, revision, "tokenizer_config.json"),
            by_name["tokenizer_config.json"],
        )
        family, architecture, entities, window, stride = _config(config, tokenizer)
        card = info.get("cardData") or {}
        license_name = card.get("license")
        for entry in catalog_models():
            if (entry.descriptor.repository, entry.descriptor.version) == (repository, revision):
                return entry.descriptor
        return ModelDescriptor.model_validate(
            {
                "name": selection_name(repository, revision),
                "version": revision,
                "repository": repository,
                "title": repository,
                "license": license_name if isinstance(license_name, str) else None,
                "model_type": family,
                "architecture": architecture,
                "entity_types": entities,
                "window_tokens": window,
                "stride_tokens": stride,
                "special_tokens": None,
                "files": artifacts,
            }
        )
    except (KeyError, TypeError, ValueError) as error:
        raise EngineError("model_metadata_invalid") from error


MESSAGE_LIMIT = 1024 * 1024


def run_management(stdin: Any, stdout: Any, root: Path) -> None:
    from .schemas import UuidString

    request_id = ""

    def reply(kind: str, payload: Any) -> None:
        data = json.dumps(
            {"id": request_id, "type": kind, "payload": payload}, separators=(",", ":")
        ).encode()
        if len(data) > MESSAGE_LIMIT:
            raise EngineError("message_too_large")
        stdout.write(data + b"\n")
        stdout.flush()

    try:
        try:
            frame = stdin.readline(MESSAGE_LIMIT + 1)
            if len(frame) > MESSAGE_LIMIT:
                raise ValueError("request too large")
            request = json.loads(frame)
            if not isinstance(request, dict) or set(request) != {"id", "type", "payload"}:
                raise ValueError("invalid envelope")
            request_id = TypeAdapter(UuidString).validate_python(request["id"], strict=True)
            kind, payload = request["type"], request["payload"]
            if kind == "check_model":
                source: ModelSource = TypeAdapter(ModelSource).validate_python(payload)
            elif kind == "install_model":
                descriptor = ModelDescriptor.model_validate(payload)
            elif kind == "remove_model":
                from .model_store import SelectionName

                name = TypeAdapter(SelectionName).validate_python(payload, strict=True)
            else:
                raise ValueError("unsupported operation")
        except (ValueError, TypeError, RecursionError) as error:
            raise EngineError("invalid_request") from error
        result: ModelDescriptor | ModelRecord
        if kind == "check_model":
            result = preflight(source, root)
        elif kind == "install_model":
            result = install_model(
                root,
                descriptor,
                request_id,
                lambda job: reply("progress", job.model_dump(mode="json")),
            )
        else:
            job = ModelJob(
                job_id=request_id,
                model_name=name,
                stage="removing",
                downloaded_bytes=0,
                total_bytes=0,
                error=None,
            )
            reply("progress", job.model_dump(mode="json"))
            result = remove_model(root, name)
            reply("progress", job.model_copy(update={"stage": "removed"}).model_dump(mode="json"))
        reply("result", result.model_dump(mode="json"))
    except EngineError as error:
        reply("error", {"code": error.code, "retryable": error.retryable})
    except PermissionError:
        reply("error", {"code": "model_store_read_only", "retryable": True})
    except Exception:
        reply("error", {"code": "model_operation_failed", "retryable": True})


def main(argv: list[str] | None = None) -> None:
    import argparse
    import sys

    parser = argparse.ArgumentParser()
    parser.add_argument("--manage-models", action="store_true")
    parser.add_argument("--model-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    run_management(sys.stdin.buffer, sys.stdout.buffer, args.model_dir)


def _local_json(path: Path) -> dict[str, Any]:
    with path.open("rb") as stream:
        data = stream.read(JSON_LIMIT + 1)
    if len(data) > JSON_LIMIT:
        raise EngineError("model_metadata_invalid")
    return _decode_json(data)


def _tokenizer_overhead(path: Path) -> int:
    from transformers import AutoTokenizer

    tokenizer = cast(Any, AutoTokenizer).from_pretrained(
        str(path), local_files_only=True, trust_remote_code=False, use_fast=True
    )
    if not tokenizer.is_fast:
        raise EngineError("model_incompatible")
    count = tokenizer.num_special_tokens_to_add(pair=False)
    if type(count) is not int or count < 0:
        raise EngineError("model_incompatible")
    return count


def _validate_downloaded(path: Path, descriptor: ModelDescriptor) -> ModelDescriptor:
    config = _local_json(path / "config.json")
    tokenizer = _local_json(path / "tokenizer_config.json")
    family, architecture, entities, tokens, _ = _config(config, tokenizer)
    if (family, architecture, entities, tokens) != (
        descriptor.model_type,
        descriptor.architecture,
        descriptor.entity_types,
        descriptor.window_tokens,
    ):
        raise EngineError("model_source_mismatch")
    try:
        with (path / "model.safetensors").open("rb") as stream:
            length = int.from_bytes(stream.read(8), "little")
            if not 0 < length <= JSON_LIMIT:
                raise ValueError("invalid weights header")
            header = _decode_json(stream.read(length))
        weight = header["classifier.weight"]["shape"]
        bias = header["classifier.bias"]["shape"]
        labels, hidden = len(config["id2label"]), config["hidden_size"]
        if (
            type(hidden) is not int
            or hidden <= 0
            or not isinstance(weight, list)
            or any(type(dimension) is not int for dimension in weight)
            or weight != [labels, hidden]
            or bias != [labels]
            or any(type(dimension) is not int for dimension in bias)
        ):
            raise ValueError("classifier dimensions")
    except (KeyError, ValueError, TypeError) as error:
        raise EngineError("model_incompatible") from error
    special = _tokenizer_overhead(path)
    expected = processing_window(
        config["max_position_embeddings"], tokenizer.get("model_max_length"), special
    )
    # Production validation uses the same loader and inference pipeline as document processing.
    actual = validate_local_model(path, descriptor.model_type, descriptor.architecture)
    if actual != expected:
        raise EngineError("model_incompatible")
    return ModelDescriptor.model_validate(
        {
            **descriptor.model_dump(),
            "window_tokens": actual.tokens,
            "stride_tokens": actual.stride,
            "special_tokens": special,
        }
    )


def _checked_path(root: Path, relative: str) -> Path:
    try:
        return _safe_path(root.absolute(), relative)
    except (OSError, ValueError) as error:
        raise EngineError("model_path_unsafe") from error


def _regular(path: Path) -> None:
    status = path.lstat()
    if (
        not stat.S_ISREG(status.st_mode)
        or status.st_nlink != 1
        or getattr(status, "st_file_attributes", 0) & 0x400
    ):
        raise EngineError("model_path_unsafe")


@contextmanager
def _native_lock(path: Path, *, create: bool, code: str) -> Iterator[Any]:
    _checked_path(path.parent, path.name)
    if path.exists():
        _regular(path)
    if os.name == "nt":
        import ctypes
        import msvcrt
        from ctypes import wintypes

        class Overlapped(ctypes.Structure):
            _fields_ = [
                ("Internal", ctypes.c_size_t),
                ("InternalHigh", ctypes.c_size_t),
                ("Offset", wintypes.DWORD),
                ("OffsetHigh", wintypes.DWORD),
                ("hEvent", wintypes.HANDLE),
            ]

        kernel = cast(Any, ctypes).WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [
            wintypes.LPCWSTR,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.c_void_p,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.HANDLE,
        ]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.LockFileEx.argtypes = [
            wintypes.HANDLE,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.POINTER(Overlapped),
        ]
        kernel.UnlockFileEx.argtypes = [
            wintypes.HANDLE,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.POINTER(Overlapped),
        ]
        # Share delete permits removal while this exclusive receipt lease remains held.
        handle = kernel.CreateFileW(
            str(path),
            0xC0000000 if create else 0x80000000,
            7,
            None,
            4 if create else 3,
            0x200080,
            None,
        )
        if handle == ctypes.c_void_p(-1).value:
            raise cast(Any, ctypes).WinError(cast(Any, ctypes).get_last_error())
        try:
            fd = cast(Any, msvcrt).open_osfhandle(handle, os.O_RDWR if create else os.O_RDONLY)
        except BaseException:
            kernel.CloseHandle(handle)
            raise
        with os.fdopen(fd, "r+b" if create else "rb") as stream:
            _regular(path)
            overlapped = Overlapped()
            if not kernel.LockFileEx(handle, 3, 0, 1, 0, ctypes.byref(overlapped)):
                raise EngineError(code, retryable=True)
            try:
                yield stream
            finally:
                kernel.UnlockFileEx(handle, 0, 1, 0, ctypes.byref(overlapped))
    else:
        import fcntl

        fd = os.open(
            path, os.O_NOFOLLOW | (os.O_RDWR | os.O_CREAT if create else os.O_RDONLY), 0o600
        )
        with os.fdopen(fd, "r+b" if create else "rb") as stream:
            _regular(path)
            if not os.path.samestat(os.fstat(fd), path.stat()):
                raise EngineError("model_path_unsafe")
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise EngineError(code, retryable=True) from error
            try:
                yield stream
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)


@contextmanager
def mutation_lock(root: Path) -> Iterator[None]:
    path = _checked_path(root, ".redactio-models-lock")
    root.mkdir(parents=True, exist_ok=True)
    with _native_lock(path, create=True, code="model_store_busy"):
        yield


def _identity(descriptor: ModelDescriptor) -> dict[str, str]:
    return {
        "name": descriptor.name,
        "version": descriptor.version,
        "repository": descriptor.repository,
    }


def _owned_directory(path: Path, descriptor: ModelDescriptor, *, staging: bool = False) -> None:
    _checked_path(path.parent, path.name)
    if not path.is_dir():
        raise EngineError("model_path_unsafe")
    allowed = {file.filename for file in descriptor.files} | {"redactio-model.json"}
    if staging:
        allowed |= {file.filename + ".part" for file in descriptor.files}
    for child in path.iterdir():
        if child.name not in allowed:
            raise EngineError("model_path_unsafe")
        _checked_path(path, child.name)
        _regular(child)
    receipt = path / "redactio-model.json"
    if not receipt.is_file() or _local_json(receipt) != _identity(descriptor):
        raise EngineError("model_path_unsafe")


def _verified_file(path: Path, artifact: Artifact) -> Artifact | None:
    if not path.exists():
        return None
    _regular(path)
    if path.stat().st_size != artifact.size:
        return None
    upstream = _hasher(artifact.upstream_hash.algorithm, artifact.size)
    sha256 = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(CHUNK):
            upstream.update(chunk)
            sha256.update(chunk)
    digest = sha256.hexdigest()
    if upstream.hexdigest() != artifact.upstream_hash.value or (
        artifact.sha256 is not None and artifact.sha256 != digest
    ):
        return None
    return artifact.model_copy(update={"sha256": digest})


def _download(
    path: Path, descriptor: ModelDescriptor, artifact: Artifact, progress: Callable[[int], None]
) -> Artifact:
    target = _checked_path(path, artifact.filename)
    partial = _checked_path(path, artifact.filename + ".part")
    if partial.exists():
        _regular(partial)
        partial.unlink()
    upstream = _hasher(artifact.upstream_hash.algorithm, artifact.size)
    sha256 = hashlib.sha256()
    count = 0
    with open_checked(
        _resolve(descriptor.repository, descriptor.version, artifact.filename)
    ) as reply:
        length = reply.getheader("Content-Length")
        if length is not None and (
            not re.fullmatch(r"[0-9]+", length) or int(length) != artifact.size
        ):
            raise EngineError("model_size_mismatch")
        with partial.open("xb") as stream:
            for chunk in _chunks(reply, artifact.size, seconds=3600):
                stream.write(chunk)
                upstream.update(chunk)
                sha256.update(chunk)
                count += len(chunk)
                if count < artifact.size:
                    progress(count)
            if count != artifact.size:
                raise EngineError("model_size_mismatch")
            digest = sha256.hexdigest()
            if upstream.hexdigest() != artifact.upstream_hash.value or (
                artifact.sha256 is not None and digest != artifact.sha256
            ):
                raise EngineError("model_hash_mismatch")
            stream.flush()
            os.fsync(stream.fileno())
    # A complete progress event means this file is durable and reusable after cancellation.
    partial.replace(target)
    progress(count)
    return artifact.model_copy(update={"sha256": digest})


def _checked_descriptor(descriptor: ModelDescriptor) -> ModelDescriptor:
    descriptor = ModelDescriptor.model_validate(descriptor.model_dump())
    if descriptor.name != selection_name(descriptor.repository, descriptor.version):
        raise EngineError("model_source_mismatch")
    _check_files(descriptor.files)
    for entry in catalog_models():
        if descriptor.name == entry.descriptor.name and descriptor != entry.descriptor:
            raise EngineError("model_source_mismatch")
    return descriptor


def install_model(
    root: Path, descriptor: ModelDescriptor, job_id: str, emit: Callable[[ModelJob], None]
) -> ModelDescriptor:
    descriptor = _checked_descriptor(descriptor)
    total = sum(file.size for file in descriptor.files)
    downloaded = 0

    def progress(stage: str, count: int) -> None:
        emit(
            ModelJob.model_validate(
                {
                    "job_id": job_id,
                    "model_name": descriptor.name,
                    "stage": stage,
                    "downloaded_bytes": count,
                    "total_bytes": total,
                    "error": None,
                }
            )
        )

    with mutation_lock(root):
        registry = read_registry(root)
        existing = next(
            (record for record in registry.models if record.descriptor.name == descriptor.name),
            None,
        )
        if any(entry.name == descriptor.name for entry in registry.legacy_unavailable):
            raise EngineError("model_path_unsafe")
        if existing is not None and existing.state == "removing":
            raise EngineError("model_store_busy", retryable=True)
        destination = _checked_path(
            root,
            existing.path
            if existing and existing.path
            else directory_name(descriptor.repository, descriptor.version),
        )
        orphan = destination.exists()
        if orphan:
            _owned_directory(destination, descriptor)
            path = destination
        else:
            path = _checked_path(
                root, "." + directory_name(descriptor.repository, descriptor.version)
            )
            if path.exists():
                _owned_directory(path, descriptor, staging=True)
            else:
                # Publish staging ownership atomically, before any model bytes can arrive.
                with tempfile.TemporaryDirectory(prefix=".model-", dir=root) as temporary:
                    staging = Path(temporary)
                    with (staging / "redactio-model.json").open("x", encoding="utf-8") as stream:
                        json.dump(_identity(descriptor), stream)
                        stream.flush()
                        os.fsync(stream.fileno())
                    staging.rename(path)
        verified = [_verified_file(path / file.filename, file) for file in descriptor.files]
        if orphan and any(file is None for file in verified):
            raise EngineError("model_hash_mismatch")
        remaining = sum(
            file.size
            for file, checked in zip(descriptor.files, verified, strict=True)
            if checked is None
        )
        if remaining and shutil.disk_usage(root).free < remaining + JSON_LIMIT:
            raise EngineError("model_insufficient_space", retryable=True)
        downloaded = total - remaining
        progress("downloading", downloaded)
        complete = []
        for artifact, checked in zip(descriptor.files, verified, strict=True):
            if checked is None:
                base = downloaded
                checked = _download(
                    path, descriptor, artifact, lambda count: progress("downloading", base + count)
                )
                downloaded += artifact.size
            complete.append(checked)
        if not orphan:
            for artifact in descriptor.files:
                partial = _checked_path(path, artifact.filename + ".part")
                if partial.exists():
                    _regular(partial)
                    partial.unlink()
        progress("validating", total)
        verified_descriptor = _validate_downloaded(
            path, descriptor.model_copy(update={"files": complete})
        )
        # The validation function returns no loaded objects, including on Windows.
        if not orphan:
            if destination.exists():
                raise EngineError("model_path_unsafe")
            path.rename(destination)
        record = ModelRecord(descriptor=verified_descriptor, path=destination.name, state="ready")
        if existing != record:
            registry.models = [
                item for item in registry.models if item.descriptor.name != descriptor.name
            ]
            registry.models.append(record)
            write_registry(root, registry)
        progress("ready", total)
        return verified_descriptor


@contextmanager
def exclusive_model_lock(root: Path, name: str) -> Iterator[None]:
    registry = read_registry(root)
    record = next((record for record in registry.models if record.descriptor.name == name), None)
    if record is None or record.path is None:
        raise EngineError("model_not_found")
    path = _checked_path(root, record.path)
    receipt = _checked_path(path, "redactio-model.json")
    with _native_lock(receipt, create=False, code="model_in_use") as stream:
        data = stream.read(JSON_LIMIT + 1)
        if len(data) > JSON_LIMIT or _decode_json(data) != _identity(record.descriptor):
            raise EngineError("model_path_unsafe")
        yield


def remove_model(root: Path, name: str) -> ModelRecord:
    with mutation_lock(root):
        registry = read_registry(root)
        record = next((item for item in registry.models if item.descriptor.name == name), None)
        if record is None:
            raise EngineError("model_not_found")
        if record.path is None:
            return record
        path = _checked_path(root, record.path)
        try:
            if path.exists():
                # A crash after deleting the last file can leave only an empty removing directory.
                if record.state == "removing" and not any(path.iterdir()):
                    path.rmdir()
                else:
                    _owned_directory(path, record.descriptor)
                    with exclusive_model_lock(root, name):
                        record.state = "removing"
                        write_registry(root, registry)
                        # Never recursively follow a directory; this store owns only flat artifacts.
                        for artifact in record.descriptor.files:
                            child = _checked_path(path, artifact.filename)
                            if child.exists():
                                _regular(child)
                                child.unlink()
                        (path / "redactio-model.json").unlink()
                        path.rmdir()
            elif record.state != "removing":
                raise EngineError("model_path_unsafe")
        except OSError as error:
            raise EngineError("model_remove_failed", retryable=True) from error
        record.path, record.state = None, "available"
        write_registry(root, registry)
        return record
