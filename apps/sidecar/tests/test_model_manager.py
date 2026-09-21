from __future__ import annotations

import errno
import hashlib
import io
import json
from contextlib import contextmanager

import pytest

from redactio_sidecar import model_manager as manager
from redactio_sidecar.ipc import EngineError
from redactio_sidecar.model_store import UrlSource

REVISION = "a" * 40
CONFIG = {
    "model_type": "bert",
    "architectures": ["BertForTokenClassification"],
    "max_position_embeddings": 512,
    "id2label": {"0": "O", "1": "B-PERSON"},
    "label2id": {"O": 0, "B-PERSON": 1},
}


@pytest.mark.parametrize(
    "url",
    [
        "http://huggingface.co/a/b",
        "https://huggingface.co.evil.invalid/a/b",
        "https://user@huggingface.co/a/b",
        "https://huggingface.co/a/b/tree/main",
        "https://huggingface.co/a/b?token=secret",
        "file:///tmp/model",
        "https://huggingface.co/a/%2e%2e",
        "https://127.0.0.1/a/b",
        "https://huggingface.co:443/a/b",
        "https://huggingface.co/a/b#",
    ],
)
def test_reject_non_repository_urls(url):
    with pytest.raises(ValueError):
        manager.parse_repository_url(url)


def test_repository_url_and_git_object_identity():
    assert manager.parse_repository_url("https://huggingface.co/one/model/") == "one/model"
    assert manager.object_digest(b"test", "git-sha1") == hashlib.sha1(b"blob 4\0test").hexdigest()
    assert manager.object_digest(b"test", "sha256") == hashlib.sha256(b"test").hexdigest()
    with pytest.raises(ValueError):
        manager.object_digest(b"test", "sha1")


class Response(io.BytesIO):
    def __init__(self, data, status=200, headers=None):
        super().__init__(data)
        self.status = status
        self.headers = headers or {}

    def read1(self, size=-1):
        return self.read(size)

    def getheader(self, name, default=None):
        return self.headers.get(name, default)


def transport(monkeypatch, replies):
    requests = []

    @contextmanager
    def connection(url):
        requests.append(url)
        value = replies(url) if callable(replies) else replies.pop(0)
        if isinstance(value, Exception):
            raise value
        yield value

    monkeypatch.setattr(manager, "_request", connection)
    return requests


def metadata(config=None):
    data = {
        "config.json": json.dumps(config or CONFIG).encode(),
        "tokenizer_config.json": b'{"model_max_length":512}',
        "tokenizer.json": b"{}",
        "model.safetensors": b"synthetic",
    }
    info = {
        "id": "one/model",
        "sha": REVISION,
        "gated": False,
        "private": False,
        "siblings": [
            {
                "rfilename": name,
                "size": len(value),
                "blobId": manager.object_digest(value, "git-sha1"),
            }
            for name, value in data.items()
        ],
    }
    return data, info


def test_preflight_uses_immutable_metadata_and_config(monkeypatch, tmp_path):
    data, info = metadata()
    requests = transport(
        monkeypatch,
        [
            Response(json.dumps(info).encode()),
            Response(json.dumps(info).encode()),
            Response(data["config.json"]),
            Response(data["tokenizer_config.json"]),
        ],
    )
    result = manager.preflight(
        UrlSource(kind="url", url="https://huggingface.co/one/model"), tmp_path
    )
    assert result.version == REVISION
    assert result.entity_types == ["PERSON"]
    assert result.special_tokens is None
    assert all(REVISION in url for url in requests[1:])
    assert not tmp_path.joinpath("manifest.json").exists()


@pytest.mark.parametrize(
    "change,code",
    [
        ({"id": "other/model"}, "model_source_mismatch"),
        ({"sha": "main"}, "model_metadata_invalid"),
        ({"gated": "auto"}, "model_access_denied"),
        ({"siblings": []}, "model_artifacts_missing"),
    ],
)
def test_preflight_rejects_untrusted_metadata(monkeypatch, tmp_path, change, code):
    _, info = metadata()
    info.update(change)
    transport(monkeypatch, lambda _: Response(json.dumps(info).encode()))
    with pytest.raises(EngineError, match=code):
        manager.preflight(UrlSource(kind="url", url="https://huggingface.co/one/model"), tmp_path)


@pytest.mark.parametrize(
    "url",
    [
        "http://huggingface.co/a",
        "https://127.0.0.1/a",
        "https://huggingface.co.evil.invalid/a",
        "https://huggingface.co:443/a",
        "https://x@huggingface.co/a",
    ],
)
def test_transport_checks_every_redirect(monkeypatch, url):
    requests = transport(monkeypatch, [Response(b"", 302, {"Location": url})])
    with pytest.raises(EngineError, match="model_network_unsafe"):
        with manager.open_checked("https://huggingface.co/a"):
            pass
    assert requests == ["https://huggingface.co/a"]


@pytest.mark.parametrize(
    "reply,code",
    [
        (Response(b"not-json"), "model_metadata_invalid"),
        (Response(b"{}", 404), "model_unavailable"),
        (TimeoutError(), "model_network_timeout"),
    ],
)
def test_bounded_json_errors_are_safe(monkeypatch, reply, code):
    transport(monkeypatch, [reply])
    with pytest.raises(EngineError, match=code):
        manager._json_url("https://huggingface.co/api/models/one/model")


@pytest.mark.parametrize(
    "address", ["127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "::ffff:127.0.0.1", "224.0.0.1"]
)
def test_connection_rejects_nonpublic_dns_before_connect(monkeypatch, address):
    monkeypatch.setattr(
        manager.socket,
        "getaddrinfo",
        lambda *a, **kw: [
            (manager.socket.AF_INET, manager.socket.SOCK_STREAM, 6, "", (address, 443))
        ],
    )
    monkeypatch.setattr(manager.socket, "socket", lambda *a: pytest.fail("unsafe socket opened"))
    with pytest.raises(EngineError, match="model_network_unsafe"):
        manager._PublicHTTPSConnection("huggingface.co").connect()


def test_connection_uses_checked_ip_and_tls_hostname(monkeypatch):
    connections = []

    class Socket:
        def settimeout(self, timeout):
            assert 0 < timeout <= 30

        def connect(self, address):
            connections.append(address)

        def getpeername(self):
            return ("1.1.1.1", 443)

        def close(self):
            pass

    class Context:
        def wrap_socket(self, raw, server_hostname):
            assert server_hostname == "huggingface.co"
            return raw

    monkeypatch.setattr(
        manager.socket,
        "getaddrinfo",
        lambda *a, **kw: [
            (manager.socket.AF_INET, manager.socket.SOCK_STREAM, 6, "", ("1.1.1.1", 443))
        ],
    )
    monkeypatch.setattr(manager.socket, "socket", lambda *a: Socket())
    connection = manager._PublicHTTPSConnection("huggingface.co")
    connection._context = Context()
    connection.connect()
    assert connections == [("1.1.1.1", 443)]


def test_response_deadline_and_json_limit(monkeypatch):
    transport(monkeypatch, [Response(b"{}")])
    ticks = iter([0, 0, 31])
    monkeypatch.setattr(manager.time, "monotonic", lambda: next(ticks))
    with pytest.raises(EngineError, match="model_network_timeout"):
        manager._json_url("https://huggingface.co/api/models/one/model")
    monkeypatch.setattr(manager.time, "monotonic", lambda: 0)
    transport(monkeypatch, [Response(b"x" * (manager.JSON_LIMIT + 1))])
    with pytest.raises(EngineError, match="model_size_mismatch"):
        manager._json_url("https://huggingface.co/api/models/one/model")


@pytest.mark.parametrize(
    "change,code",
    [
        ({"auto_map": {"AutoConfig": "code.Config"}}, "model_remote_code_unsupported"),
        ({"architectures": ["BertModel"]}, "model_incompatible"),
        ({"id2label": {"0": "LABEL_0", "1": "LABEL_1"}}, "model_labels_invalid"),
        ({"id2label": {"0": "O", "1": "B-invalid"}}, "model_labels_invalid"),
        ({"label2id": {"O": 1, "B-PERSON": 0}}, "model_labels_invalid"),
    ],
)
def test_incompatible_configs_are_rejected(monkeypatch, tmp_path, change, code):
    data, info = metadata({**CONFIG, **change})
    transport(
        monkeypatch,
        [
            Response(json.dumps(info).encode()),
            Response(json.dumps(info).encode()),
            Response(data["config.json"]),
            Response(data["tokenizer_config.json"]),
        ],
    )
    with pytest.raises(EngineError, match=code):
        manager.preflight(UrlSource(kind="url", url="https://huggingface.co/one/model"), tmp_path)


@pytest.mark.parametrize(
    "change",
    [
        {"size": True},
        {"size": -1},
        {"blobId": "a" * 64},
        {"size": manager.WEIGHT_LIMIT + 1},
        {"lfs": {"size": 123, "sha256": "a" * 64}},
    ],
)
def test_bad_artifact_metadata_is_rejected(monkeypatch, tmp_path, change):
    _, info = metadata()
    info["siblings"][-1].update(change)
    transport(monkeypatch, lambda _: Response(json.dumps(info).encode()))
    with pytest.raises(EngineError):
        manager.preflight(UrlSource(kind="url", url="https://huggingface.co/one/model"), tmp_path)


def test_catalog_preflight_is_offline_and_preserves_readme(monkeypatch, tmp_path):
    from redactio_sidecar.model_store import CatalogSource

    transport(monkeypatch, lambda _: pytest.fail("catalog used network"))
    descriptor = manager.preflight(CatalogSource(kind="catalog", key="biomedbert"), tmp_path)
    assert "README.md" in {file.filename for file in descriptor.files}
    assert all(file.sha256 for file in descriptor.files)


def test_management_envelope_has_one_bounded_request_and_no_engine(tmp_path):
    request_id = "00000000-0000-4000-8000-000000000001"
    request = {
        "id": request_id,
        "type": "check_model",
        "payload": {"kind": "catalog", "key": "biomedbert"},
    }
    output = io.BytesIO()
    manager.run_management(io.BytesIO(json.dumps(request).encode() + b"\n"), output, tmp_path)
    reply = json.loads(output.getvalue())
    assert reply["id"] == request_id and reply["type"] == "result"
    assert reply["payload"]["name"] == manager.catalog_models()[0].descriptor.name
    assert not (tmp_path / "manifest.json").exists()


@pytest.mark.parametrize(
    "frame",
    [
        b"{",
        b"{}",
        b"[]",
        b"x" * (1024 * 1024 + 1),
        b'{"id":"00000000-0000-4000-8000-000000000001","type":"process_document","payload":{}}',
        b'{"id":"00000000-0000-4000-8000-000000000001","type":"check_model","payload":{"kind":"catalog","key":"biomedbert"},"extra":1}',
    ],
)
def test_management_rejects_invalid_envelopes_without_details(tmp_path, frame):
    output = io.BytesIO()
    manager.run_management(io.BytesIO(frame + b"\n"), output, tmp_path)
    response = json.loads(output.getvalue())
    assert response["type"] == "error"
    assert response["payload"] == {"code": "invalid_request", "retryable": False}


def install_fixture(monkeypatch):
    from redactio_sidecar.model_store import Artifact, ModelDescriptor, UpstreamHash, Window

    data, _ = metadata()
    header = json.dumps(
        {"classifier.weight": {"shape": [2, 3]}, "classifier.bias": {"shape": [2]}}
    ).encode()
    data["model.safetensors"] = len(header).to_bytes(8, "little") + header
    data["config.json"] = json.dumps({**CONFIG, "hidden_size": 3}).encode()
    descriptor = ModelDescriptor(
        name=f"hf:one/model@{REVISION}",
        repository="one/model",
        version=REVISION,
        title="One",
        license=None,
        model_type="bert",
        architecture="BertForTokenClassification",
        entity_types=["PERSON"],
        window_tokens=512,
        stride_tokens=128,
        special_tokens=None,
        files=[
            Artifact(
                filename=name,
                size=len(value),
                upstream_hash=UpstreamHash(
                    algorithm="git-sha1", value=manager.object_digest(value, "git-sha1")
                ),
                sha256=None,
            )
            for name, value in data.items()
        ],
    )
    monkeypatch.setattr(manager, "validate_local_model", lambda *a: Window(512, 128))
    monkeypatch.setattr(manager, "_tokenizer_overhead", lambda path: 2)
    return descriptor, data


JOB = "00000000-0000-4000-8000-000000000001"


@pytest.mark.parametrize("catalog_index", [0, 1])
@pytest.mark.parametrize("migrated", [False, True])
def test_reinstall_missing_catalog_legacy_preserves_other_entries(
    monkeypatch, tmp_path, catalog_index, migrated
):
    descriptor = manager.catalog_models()[catalog_index].descriptor
    legacy = {"name": descriptor.name, "version": descriptor.version, "path": "old-model"}
    unrelated = {"name": "unknown-model", "version": "old", "path": "unowned"}
    manifest = tmp_path / "manifest.json"
    manifest.write_text(json.dumps({"models": [legacy, unrelated]}))
    unowned = tmp_path / "unowned"
    unowned.mkdir()
    (unowned / "personal.txt").write_text("keep")
    if migrated:
        other, data = install_fixture(monkeypatch)
        transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
        manager.install_model(tmp_path, other, JOB, lambda _: None)
        assert json.loads(manifest.read_text())["schema_version"] == 2
    before = manager.read_registry(tmp_path)
    assert legacy in [entry.model_dump() for entry in before.legacy_unavailable]
    before_bytes = manifest.read_bytes()

    def downloaded(path, selected, artifact, progress):
        (path / artifact.filename).write_bytes(b"synthetic catalog artifact")
        return artifact

    monkeypatch.setattr(manager, "_download", downloaded)
    monkeypatch.setattr(manager, "_validate_downloaded", lambda path, value: value)

    def cancel(job):
        if job.stage == "validating":
            raise KeyboardInterrupt()

    with pytest.raises(KeyboardInterrupt):
        manager.install_model(tmp_path, descriptor, JOB, cancel)
    assert manifest.read_bytes() == before_bytes
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    registry = manager.read_registry(tmp_path)
    assert registry.models[:-1] == before.models
    assert registry.models[-1].descriptor == descriptor
    assert registry.models[-1].state == "ready"
    assert registry.models[-1].path == manager.directory_name(
        descriptor.repository, descriptor.version
    )
    assert [entry.model_dump() for entry in registry.legacy_unavailable] == [unrelated]
    assert (unowned / "personal.txt").read_text() == "keep"
    assert not (tmp_path / "old-model").exists()


@pytest.mark.parametrize("recognized", [False, True])
def test_legacy_reinstall_preserves_unowned_destination(monkeypatch, tmp_path, recognized):
    descriptor = manager.catalog_models()[0].descriptor
    destination = tmp_path / (
        manager.directory_name(descriptor.repository, descriptor.version)
        if recognized
        else "old-model"
    )
    destination.mkdir()
    (destination / "personal.txt").write_text("keep")
    before = json.dumps(
        {
            "models": [
                {
                    "name": descriptor.name,
                    "version": descriptor.version if recognized else "unknown",
                    "path": "old-model",
                }
            ]
        }
    )
    (tmp_path / "manifest.json").write_text(before)
    transport(monkeypatch, lambda _: pytest.fail("unsafe legacy installation downloaded"))
    with pytest.raises(EngineError, match="model_path_unsafe"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert (destination / "personal.txt").read_text() == "keep"
    assert (tmp_path / "manifest.json").read_text() == before


@pytest.mark.parametrize("operation", ["open", "write", "flush", "fsync"])
@pytest.mark.parametrize(
    "error,code",
    [
        (PermissionError(errno.EACCES, "private path"), "model_store_read_only"),
        (OSError(errno.EROFS, "private path"), "model_store_read_only"),
        (OSError(errno.ENOSPC, "private path"), "model_insufficient_space"),
    ],
)
def test_download_destination_failure_is_storage_error(
    monkeypatch, tmp_path, operation, error, code
):
    descriptor, data = install_fixture(monkeypatch)
    artifact = descriptor.files[0]
    transport(monkeypatch, lambda _: Response(data[artifact.filename]))
    original_open = manager.Path.open

    class FailedDestination(io.BytesIO):
        def write(self, value):
            if operation == "write":
                raise error
            return super().write(value)

        def flush(self):
            if operation == "flush":
                raise error

        def fileno(self):
            return 0

    def failed_open(path, *args, **kwargs):
        if path.suffix == ".part":
            if operation == "open":
                raise error
            return FailedDestination()
        return original_open(path, *args, **kwargs)

    def failed_fsync(_):
        raise error

    monkeypatch.setattr(manager.Path, "open", failed_open)
    if operation == "fsync":
        monkeypatch.setattr(manager.os, "fsync", failed_fsync)
    with pytest.raises(EngineError, match=code) as failure:
        manager._download(tmp_path, descriptor, artifact, lambda _: None)
    assert failure.value.retryable
    assert not (tmp_path / artifact.filename).exists()
    assert not manager.read_registry(tmp_path).models


@pytest.mark.parametrize(
    "error,code",
    [
        (TimeoutError(), "model_network_timeout"),
        (ConnectionResetError(), "model_network_failed"),
    ],
)
def test_download_interrupted_read_remains_network_error(monkeypatch, tmp_path, error, code):
    descriptor, data = install_fixture(monkeypatch)
    artifact = descriptor.files[0]

    class Interrupted(Response):
        def read1(self, size=-1):
            if self.tell():
                raise error
            return super().read1(1)

    transport(monkeypatch, lambda _: Interrupted(data[artifact.filename]))
    with pytest.raises(EngineError, match=code):
        manager._download(tmp_path, descriptor, artifact, lambda _: None)
    assert not (tmp_path / artifact.filename).exists()
    assert (tmp_path / (artifact.filename + ".part")).read_bytes() == data[artifact.filename][:1]


def test_install_pins_revision_hashes_files_and_publishes_last(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    requests = transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    progress = []
    result = manager.install_model(tmp_path, descriptor, JOB, progress.append)
    registry = manager.read_registry(tmp_path)
    assert result.special_tokens == 2 and all(file.sha256 for file in result.files)
    assert registry.models[0].descriptor == result and registry.models[0].state == "ready"
    assert all(f"/resolve/{REVISION}/" in url for url in requests)
    assert [p.downloaded_bytes for p in progress] == sorted(p.downloaded_bytes for p in progress)
    assert all(p.job_id == JOB for p in progress)
    assert progress[-1].stage == "ready"
    transport(monkeypatch, lambda _: pytest.fail("retry used network"))
    assert manager.install_model(tmp_path, descriptor, JOB, lambda _: None) == result


@pytest.mark.parametrize("corrupt", [b"wrong", b"", b"x" * 10000])
def test_failed_transfer_preserves_manifest_and_retry_restarts_file(monkeypatch, tmp_path, corrupt):
    descriptor, data = install_fixture(monkeypatch)
    before = b'{"models":[]}\n'
    (tmp_path / "manifest.json").write_bytes(before)
    transport(monkeypatch, lambda _: Response(corrupt))
    with pytest.raises(EngineError, match="model_(hash|size)_mismatch"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert (tmp_path / "manifest.json").read_bytes() == before
    assert not manager.read_registry(tmp_path).models
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    assert manager.install_model(tmp_path, descriptor, JOB, lambda _: None).special_tokens == 2


def test_retry_after_rename_and_failed_registry_write_is_offline(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    original = manager.write_registry
    monkeypatch.setattr(
        manager, "write_registry", lambda *a: (_ for _ in ()).throw(OSError("disk"))
    )
    with pytest.raises(OSError):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert not manager.read_registry(tmp_path).models
    monkeypatch.setattr(manager, "write_registry", original)
    transport(monkeypatch, lambda _: pytest.fail("orphan retry used network"))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert manager.read_registry(tmp_path).models[0].state == "ready"


def test_validation_failure_and_cancellation_leave_recoverable_staging(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))

    def cancel(*args):
        raise KeyboardInterrupt()

    monkeypatch.setattr(manager, "validate_local_model", cancel)
    with pytest.raises(KeyboardInterrupt):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert not manager.read_registry(tmp_path).models
    assert list(tmp_path.glob(".model-*"))
    from redactio_sidecar.model_store import Window

    monkeypatch.setattr(manager, "validate_local_model", lambda *a: Window(512, 128))
    transport(monkeypatch, lambda _: pytest.fail("complete staged file redownloaded"))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)


def test_unowned_destination_and_linked_stage_are_preserved(monkeypatch, tmp_path):
    descriptor, _ = install_fixture(monkeypatch)
    from redactio_sidecar.model_store import directory_name

    destination = tmp_path / directory_name(descriptor.repository, descriptor.version)
    destination.mkdir()
    marker = destination / "personal.txt"
    marker.write_text("keep")
    with pytest.raises(EngineError, match="model_path_unsafe"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert marker.read_text() == "keep"


def test_remove_retains_receipt_and_failed_deletion_is_retryable(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    record = manager.read_registry(tmp_path).models[0]
    path = tmp_path / record.path
    original = manager.Path.unlink

    def fail_weights(self, *a, **kw):
        if self.name == "model.safetensors":
            raise PermissionError("busy")
        return original(self, *a, **kw)

    monkeypatch.setattr(manager.Path, "unlink", fail_weights)
    with pytest.raises(EngineError, match="model_remove_failed"):
        manager.remove_model(tmp_path, descriptor.name)
    assert manager.read_registry(tmp_path).models[0].state == "removing"
    assert (path / "model.safetensors").exists()
    assert (path / "redactio-model.json").exists()
    monkeypatch.setattr(manager.Path, "unlink", original)
    result = manager.remove_model(tmp_path, descriptor.name)
    assert result.state == "available" and result.path is None
    assert not path.exists()
    assert (
        manager.preflight(
            manager.TypeAdapter(manager.ModelSource).validate_python(
                {"kind": "receipt", "name": descriptor.name}
            ),
            tmp_path,
        ).version
        == REVISION
    )


@pytest.mark.parametrize("entry", ["module", "frozen"])
def test_entrypoint_routes_management_before_document_engine(tmp_path, entry):
    import subprocess
    import sys
    from pathlib import Path

    args = (
        [sys.executable, "-m", "redactio_sidecar"]
        if entry == "module"
        else [
            sys.executable,
            str(Path(__file__).resolve().parents[3] / "packaging/sidecar-entry.py"),
        ]
    )
    frame = {"id": JOB, "type": "check_model", "payload": {"kind": "catalog", "key": "biomedbert"}}
    result = subprocess.run(
        [*args, "--manage-models", "--model-dir", str(tmp_path)],
        input=json.dumps(frame) + "\n",
        text=True,
        capture_output=True,
        timeout=10,
    )
    assert result.returncode == 0, result.stderr
    response = json.loads(result.stdout)
    assert response["type"] == "result" and response["id"] == JOB


def test_metadata_rejects_duplicate_json_fields(monkeypatch):
    transport(monkeypatch, [Response(b'{"sha":"a","sha":"b"}')])
    with pytest.raises(EngineError, match="model_metadata_invalid"):
        manager._json_url("https://huggingface.co/api/models/one/model")


def test_preflight_rejects_ambiguous_lfs_size_types(monkeypatch, tmp_path):
    _, info = metadata()
    info["siblings"][-1].update({"size": 1, "lfs": {"size": True, "sha256": "a" * 64}})
    transport(monkeypatch, lambda _: Response(json.dumps(info).encode()))
    with pytest.raises(EngineError, match="model_metadata_invalid"):
        manager.preflight(UrlSource(kind="url", url="https://huggingface.co/one/model"), tmp_path)


def test_native_label_indices_cannot_be_booleans():
    with pytest.raises(EngineError, match="model_labels_invalid"):
        manager._config({**CONFIG, "label2id": {"O": False, "B-PERSON": True}}, {})


@pytest.mark.parametrize("shape", [[1, 3], [2, 999], [True, 3], [2], []])
def test_classifier_dimensions_checked_before_runtime(monkeypatch, tmp_path, shape):
    descriptor, data = install_fixture(monkeypatch)
    header = json.dumps(
        {"classifier.weight": {"shape": shape}, "classifier.bias": {"shape": [2]}}
    ).encode()
    data["model.safetensors"] = len(header).to_bytes(8, "little") + header
    for filename, value in data.items():
        (tmp_path / filename).write_bytes(value)
    monkeypatch.setattr(
        manager, "validate_local_model", lambda *a: pytest.fail("unsafe model loaded")
    )
    with pytest.raises(EngineError, match="model_incompatible"):
        manager._validate_downloaded(tmp_path, descriptor)


def test_actual_configuration_must_match_checked_descriptor(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    data["config.json"] = json.dumps({**CONFIG, "max_position_embeddings": 256}).encode()
    for filename, value in data.items():
        (tmp_path / filename).write_bytes(value)
    monkeypatch.setattr(
        manager, "validate_local_model", lambda *a: pytest.fail("changed model loaded")
    )
    with pytest.raises(EngineError, match="model_source_mismatch"):
        manager._validate_downloaded(tmp_path, descriptor)


def test_native_mutation_lock_contends_and_survives_failed_acquisition(tmp_path):
    import subprocess
    import sys

    command = """from pathlib import Path
from redactio_sidecar.model_manager import mutation_lock
from redactio_sidecar.ipc import EngineError
import sys
try:
    with mutation_lock(Path(sys.argv[1])):
        print('acquired')
except EngineError as error:
    print(error.code)
"""
    with manager.mutation_lock(tmp_path):
        result = subprocess.run(
            [sys.executable, "-c", command, str(tmp_path)],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.stdout.strip() == "model_store_busy", result.stderr
    result = subprocess.run(
        [sys.executable, "-c", command, str(tmp_path)], capture_output=True, text=True, timeout=10
    )
    assert result.stdout.strip() == "acquired", result.stderr
    assert (tmp_path / ".redactio-models-lock").is_file()


def test_native_receipt_handle_can_read_and_delete_while_locked(tmp_path):
    model = tmp_path / "model"
    model.mkdir()
    receipt = model / "redactio-model.json"
    receipt.write_bytes(b'{"identity":"synthetic"}')
    with manager._native_lock(receipt, create=False, code="model_in_use") as stream:
        assert stream.read() == b'{"identity":"synthetic"}'
        receipt.unlink()
        model.rmdir()
    assert not model.exists()


def test_mutation_lock_refuses_symlink(tmp_path):
    other = tmp_path / "other"
    other.write_text("untouched")
    (tmp_path / ".redactio-models-lock").symlink_to(other)
    with pytest.raises(EngineError, match="model_path_unsafe"):
        with manager.mutation_lock(tmp_path):
            pytest.fail("linked lock accepted")
    assert other.read_text() == "untouched"


def test_dns_resolution_is_bounded(monkeypatch):
    import threading

    waiting = threading.Event()
    monkeypatch.setattr(manager, "TIMEOUT", 0.01)
    monkeypatch.setattr(manager.socket, "getaddrinfo", lambda *a, **kw: waiting.wait(2))
    try:
        with pytest.raises(TimeoutError):
            manager._PublicHTTPSConnection("huggingface.co").connect()
    finally:
        waiting.set()


def test_cancel_after_complete_file_reuses_only_verified_file(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    requests = transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))

    def cancel(job):
        if job.downloaded_bytes == descriptor.files[0].size:
            raise KeyboardInterrupt()

    with pytest.raises(KeyboardInterrupt):
        manager.install_model(tmp_path, descriptor, JOB, cancel)
    assert not manager.read_registry(tmp_path).models
    requests.clear()
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert all(not url.endswith("/config.json") for url in requests)


def test_removal_rejects_symlink_and_preserves_target(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    record = manager.read_registry(tmp_path).models[0]
    outside = tmp_path / "outside"
    outside.write_text("personal")
    weights = tmp_path / record.path / "model.safetensors"
    weights.unlink()
    weights.symlink_to(outside)
    with pytest.raises(EngineError, match="model_path_unsafe"):
        manager.remove_model(tmp_path, descriptor.name)
    assert outside.read_text() == "personal"
    assert manager.read_registry(tmp_path).models[0].state == "ready"


def test_disk_space_failure_does_not_download(monkeypatch, tmp_path):
    descriptor, _ = install_fixture(monkeypatch)
    monkeypatch.setattr(manager.shutil, "disk_usage", lambda _: type("Usage", (), {"free": 0})())
    transport(monkeypatch, lambda _: pytest.fail("disk-full download"))
    with pytest.raises(EngineError, match="model_insufficient_space"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert not manager.read_registry(tmp_path).models


@pytest.mark.parametrize(
    "url", ["https://huggingface.co/one/model\n", "https://huggingface.co/one/mo\tdel"]
)
def test_repository_parser_does_not_normalize_control_characters(url):
    with pytest.raises(ValueError):
        manager.parse_repository_url(url)


def test_catalog_hashes_cannot_be_replaced_by_request(monkeypatch, tmp_path):
    descriptor = manager.catalog_models()[0].descriptor.model_copy(deep=True)
    descriptor.files[0].sha256 = "0" * 64
    transport(monkeypatch, lambda _: pytest.fail("altered catalog downloaded"))
    with pytest.raises(EngineError, match="model_source_mismatch"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)


def test_url_to_catalog_revision_retains_authoritative_catalog(monkeypatch, tmp_path):
    from redactio_sidecar.model_store import CatalogSource

    catalog = manager.preflight(CatalogSource(kind="catalog", key="biomedbert"), tmp_path)
    data, info = metadata()
    info.update(id=catalog.repository, sha=catalog.version)
    transport(
        monkeypatch,
        [
            Response(json.dumps(info).encode()),
            Response(json.dumps(info).encode()),
            Response(data["config.json"]),
            Response(data["tokenizer_config.json"]),
        ],
    )
    result = manager.preflight(
        UrlSource(kind="url", url=f"https://huggingface.co/{catalog.repository}"), tmp_path
    )
    assert result == catalog


def test_retry_cleans_only_owned_partial_files_before_publication(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))

    def cancel(job):
        if job.stage == "validating":
            raise KeyboardInterrupt()

    with pytest.raises(KeyboardInterrupt):
        manager.install_model(tmp_path, descriptor, JOB, cancel)
    stage = next(tmp_path.glob(".model-*"))
    (stage / "config.json.part").write_bytes(b"old incomplete transfer")
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    record = manager.read_registry(tmp_path).models[0]
    assert not (tmp_path / record.path / "config.json.part").exists()
    assert manager.remove_model(tmp_path, descriptor.name).state == "available"


def test_worker_reports_read_only_without_exception_details(monkeypatch, tmp_path):
    def denied(*args):
        raise PermissionError("private path must not appear")

    monkeypatch.setattr(manager, "remove_model", denied)
    frame = {"id": JOB, "type": "remove_model", "payload": "synthetic"}
    output = io.BytesIO()
    manager.run_management(io.BytesIO(json.dumps(frame).encode()), output, tmp_path)
    response = json.loads(output.getvalue().splitlines()[-1])
    assert response["payload"] == {"code": "model_store_read_only", "retryable": True}


def test_remove_worker_progress_uses_request_job_id(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    frame = {"id": JOB, "type": "remove_model", "payload": descriptor.name}
    output = io.BytesIO()
    manager.run_management(io.BytesIO(json.dumps(frame).encode()), output, tmp_path)
    messages = [json.loads(line) for line in output.getvalue().splitlines()]
    assert [message["type"] for message in messages] == ["progress", "progress", "result"]
    assert [message["payload"]["stage"] for message in messages[:2]] == ["removing", "removed"]
    assert all(message["payload"]["job_id"] == JOB for message in messages[:2])


def test_interrupted_staging_receipt_creation_does_not_poison_retry(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    original = manager.json.dump

    def interrupt(value, stream):
        stream.write("{")
        raise KeyboardInterrupt()

    monkeypatch.setattr(manager.json, "dump", interrupt)
    with pytest.raises(KeyboardInterrupt):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    monkeypatch.setattr(manager.json, "dump", original)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert manager.read_registry(tmp_path).models[0].state == "ready"


def test_validation_failure_preserves_previous_ready_model_and_manifest(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    before = (tmp_path / "manifest.json").read_bytes()
    old_record = manager.read_registry(tmp_path).models[0]
    old_weights = (tmp_path / old_record.path / "model.safetensors").read_bytes()
    newer = descriptor.model_copy(update={"version": "b" * 40, "name": "hf:one/model@" + "b" * 40})

    def invalid(*args):
        raise EngineError("model_incompatible")

    monkeypatch.setattr(manager, "validate_local_model", invalid)
    with pytest.raises(EngineError, match="model_incompatible"):
        manager.install_model(tmp_path, newer, JOB, lambda _: None)
    assert (tmp_path / "manifest.json").read_bytes() == before
    assert (tmp_path / old_record.path / "model.safetensors").read_bytes() == old_weights


def test_failed_final_removal_registry_write_can_be_retried(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    original = manager.write_registry

    def fail_available(root, registry):
        if registry.models[0].state == "available":
            raise OSError("disk error")
        original(root, registry)

    monkeypatch.setattr(manager, "write_registry", fail_available)
    with pytest.raises(OSError):
        manager.remove_model(tmp_path, descriptor.name)
    record = manager.read_registry(tmp_path).models[0]
    assert record.state == "removing" and not (tmp_path / record.path).exists()
    monkeypatch.setattr(manager, "write_registry", original)
    assert manager.remove_model(tmp_path, descriptor.name).state == "available"


@pytest.mark.parametrize("length", ["0", "-1", "999999", "garbage"])
def test_declared_content_length_must_match_checked_artifact(monkeypatch, tmp_path, length):
    descriptor, data = install_fixture(monkeypatch)
    transport(
        monkeypatch,
        lambda url: Response(data[url.rsplit("/", 1)[1]], headers={"Content-Length": length}),
    )
    with pytest.raises(EngineError, match="model_size_mismatch"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert not manager.read_registry(tmp_path).models


def test_linked_staging_directory_is_not_used(monkeypatch, tmp_path):
    descriptor, _ = install_fixture(monkeypatch)
    from redactio_sidecar.model_store import directory_name

    outside = tmp_path / "personal"
    outside.mkdir()
    (outside / "keep.txt").write_text("keep")
    stage = tmp_path / ("." + directory_name(descriptor.repository, descriptor.version))
    stage.symlink_to(outside, target_is_directory=True)
    transport(monkeypatch, lambda _: pytest.fail("linked stage download"))
    with pytest.raises(EngineError, match="model_path_unsafe"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert (outside / "keep.txt").read_text() == "keep"


def test_future_registry_is_unchanged_by_install(monkeypatch, tmp_path):
    descriptor, _ = install_fixture(monkeypatch)
    before = b'{"schema_version":999,"models":[],"legacy_unavailable":[]}'
    (tmp_path / "manifest.json").write_bytes(before)
    transport(monkeypatch, lambda _: pytest.fail("future registry download"))
    with pytest.raises(EngineError, match="invalid_model_manifest"):
        manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    assert (tmp_path / "manifest.json").read_bytes() == before
    assert not list(tmp_path.glob(".model-*"))


@pytest.mark.skipif(
    manager.os.name == "nt", reason="Rust/Windows shared lease proof runs in Task 4"
)
def test_shared_inference_receipt_blocks_removal_process(monkeypatch, tmp_path):
    import fcntl
    import subprocess
    import sys

    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    record = manager.read_registry(tmp_path).models[0]
    with (tmp_path / record.path / "redactio-model.json").open("rb") as stream:
        fcntl.flock(stream, fcntl.LOCK_SH)
        child = subprocess.run(
            [
                sys.executable,
                "-m",
                "redactio_sidecar",
                "--manage-models",
                "--model-dir",
                str(tmp_path),
            ],
            input=json.dumps({"id": JOB, "type": "remove_model", "payload": descriptor.name})
            + "\n",
            capture_output=True,
            text=True,
            timeout=10,
        )
        response = json.loads(child.stdout.splitlines()[-1])
        assert response["type"] == "error" and response["payload"]["code"] == "model_in_use"
        assert manager.read_registry(tmp_path).models[0].state == "ready"
    assert manager.remove_model(tmp_path, descriptor.name).state == "available"


@pytest.mark.parametrize(
    "options",
    [
        {"tokenizer_file": "/outside/tokenizer.json"},
        {"vocab_file": "../vocab.txt"},
        {"from_slow": True},
        {"tokenizer_class": "GPT2Tokenizer"},
        {"sp_model_kwargs": {"model_file": "/outside/spm.model"}},
    ],
)
def test_preflight_rejects_tokenizer_loader_controls(options):
    with pytest.raises(EngineError, match="model_incompatible"):
        manager._config(CONFIG, options)


def test_retry_reclaims_owned_partial_before_measuring_free_space(monkeypatch, tmp_path):
    descriptor, data = install_fixture(monkeypatch)
    transport(monkeypatch, lambda url: Response(data[url.rsplit("/", 1)[1]]))

    def cancel(job):
        if job.downloaded_bytes == descriptor.files[0].size:
            raise KeyboardInterrupt()

    with pytest.raises(KeyboardInterrupt):
        manager.install_model(tmp_path, descriptor, JOB, cancel)
    stage = next(tmp_path.glob(".model-*"))
    partial = stage / "model.safetensors.part"
    partial.write_bytes(b"incomplete transfer")
    complete = (stage / "config.json").read_bytes()
    monkeypatch.setattr(
        manager.shutil,
        "disk_usage",
        lambda _: type(
            "Usage", (), {"free": 0 if partial.exists() else 100 * manager.JSON_LIMIT}
        )(),
    )
    manager.install_model(tmp_path, descriptor, JOB, lambda _: None)
    record = manager.read_registry(tmp_path).models[0]
    assert (tmp_path / record.path / "config.json").read_bytes() == complete
    assert record.state == "ready"
