"""Synthetic worker lifetime proof; runnable with unittest on pinned native Python."""

from __future__ import annotations

import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import contextmanager
from pathlib import Path
from unittest.mock import patch
from uuid import uuid4

from redactio_sidecar import biomedbert, model_manager, model_store
from redactio_sidecar import engine as engine_module
from redactio_sidecar.engine import Engine
from redactio_sidecar.ipc import EngineError
from redactio_sidecar.model_lock import _native_lock
from redactio_sidecar.schemas import ProcessingConfig


def fixture_store(root: Path) -> list[model_store.ModelRecord]:
    records = []
    for index in range(2):
        repository, revision = f"synthetic/model{index}", "a" * 40
        name = model_store.selection_name(repository, revision)
        directory = model_store.directory_name(repository, revision)
        path = root / directory
        path.mkdir()
        content = {
            "config.json": json.dumps(
                {
                    "model_type": "bert",
                    "architectures": ["BertForTokenClassification"],
                    "max_position_embeddings": 512,
                    "id2label": {"0": "O", "1": "B-PERSON"},
                }
            ).encode(),
            "tokenizer_config.json": b'{"model_max_length":512}',
            "tokenizer.json": b"{}",
            "model.safetensors": b"synthetic",
        }
        artifacts = []
        for filename, data in content.items():
            (path / filename).write_bytes(data)
            digest = hashlib.sha256(data).hexdigest()
            artifacts.append(
                model_store.Artifact(
                    filename=filename,
                    size=len(data),
                    sha256=digest,
                    upstream_hash=model_store.UpstreamHash(algorithm="sha256", value=digest),
                )
            )
        (path / "redactio-model.json").write_text(
            json.dumps(
                {
                    "name": name,
                    "version": revision,
                    "repository": repository,
                }
            )
        )
        records.append(
            model_store.ModelRecord(
                descriptor=model_store.ModelDescriptor(
                    name=name,
                    version=revision,
                    repository=repository,
                    title="Synthetic",
                    license=None,
                    model_type="bert",
                    architecture="BertForTokenClassification",
                    entity_types=["PERSON"],
                    window_tokens=512,
                    stride_tokens=128,
                    special_tokens=2,
                    files=artifacts,
                ),
                path=directory,
                state="ready",
            )
        )
    model_store.write_registry(
        root,
        model_store.ModelRegistry(
            schema_version=2,
            models=records,
            legacy_unavailable=[],
        ),
    )
    return records


def configure(engine: Engine, name: str) -> None:
    engine.configure(
        str(uuid4()),
        str(uuid4()),
        ProcessingConfig(
            model=name,
            model_entities=[],
            enabled_entities=[],
        ),
    )


def removal(root: Path, name: str) -> dict:
    result = subprocess.run(
        [sys.executable, "-m", "redactio_sidecar", "--manage-models", "--model-dir", str(root)],
        input=json.dumps({"id": str(uuid4()), "type": "remove_model", "payload": name}) + "\n",
        text=True,
        capture_output=True,
        timeout=20,
        check=True,
    )
    return json.loads(result.stdout.splitlines()[-1])


class EngineLeaseTests(unittest.TestCase):
    def test_cached_idle_models_survive_launcher_death_until_worker_exit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            records = fixture_store(root)
            ready = root / "worker-ready.json"
            command = [sys.executable, str(Path(__file__).resolve()), "--worker", str(root)]
            launcher = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import subprocess,sys; subprocess.Popen(sys.argv[1:]).wait()",
                    *command,
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
            )
            worker_pid = None
            try:
                deadline = time.monotonic() + 30
                while not ready.exists() and time.monotonic() < deadline:
                    self.assertIsNone(launcher.poll())
                    time.sleep(0.05)
                self.assertTrue(ready.exists(), "synthetic Engine child did not become ready")
                worker_pid = json.loads(ready.read_text())["pid"]
                launcher.kill()
                launcher.wait(timeout=10)
                for record in records:
                    response = removal(root, record.descriptor.name)
                    self.assertEqual(response["type"], "error")
                    self.assertEqual(
                        response["payload"], {"code": "model_in_use", "retryable": True}
                    )
                os.kill(worker_pid, signal.SIGTERM)
                worker_pid = None
                deadline = time.monotonic() + 10
                while True:
                    response = removal(root, records[0].descriptor.name)
                    if response["type"] == "result" or time.monotonic() >= deadline:
                        break
                    time.sleep(0.05)
                self.assertEqual(response["type"], "result")
                self.assertEqual(removal(root, records[1].descriptor.name)["type"], "result")
            finally:
                if worker_pid is not None:
                    os.kill(worker_pid, signal.SIGTERM)
                if launcher.poll() is None:
                    launcher.kill()
                    launcher.wait(timeout=10)
                launcher.stderr.close()

    def test_shared_leases_coexist_and_exclusive_acquisition_blocks_engine(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            records = fixture_store(root)
            engine = Engine(root)
            # Discover first: Windows forbids reading another handle's exclusive range.
            model = engine._models()[0]
            with model_manager.exclusive_model_lock(root, records[0].descriptor.name):
                with self.assertRaisesRegex(EngineError, "model_in_use"):
                    engine._nlp_engine(model)
            with patch.object(biomedbert, "_load_pipeline", return_value=lambda _: []):
                configure(engine, records[0].descriptor.name)
                other = Engine(root)
                configure(other, records[0].descriptor.name)
            with self.assertRaisesRegex(EngineError, "model_in_use"):
                with model_manager.exclusive_model_lock(root, records[0].descriptor.name):
                    self.fail("cached shared locks must block exclusive removal")
            del engine, other
            self.assertEqual(removal(root, records[0].descriptor.name)["type"], "result")

    def test_replacement_receipt_and_changed_ready_record_are_rejected(self):
        for change in ("receipt", "record"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                records = fixture_store(root)
                engine = Engine(root)

                @contextmanager
                def replace_after_acquisition(path, **kwargs):
                    with _native_lock(path, **kwargs) as stream:
                        if change == "receipt":
                            replacement = root / "replacement.json"
                            replacement.write_bytes(path.read_bytes())
                            path.rename(root / "obsolete.json")
                            replacement.rename(path)
                        else:
                            registry = model_store.read_registry(root)
                            registry.models[0].state = "removing"
                            model_store.write_registry(root, registry)
                        yield stream

                with patch.object(engine_module, "_native_lock", replace_after_acquisition):
                    with self.assertRaises(EngineError):
                        configure(engine, records[0].descriptor.name)
                self.assertEqual(removal(root, records[0].descriptor.name)["type"], "result")

    def test_legacy_store_lease_needs_no_writes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            record = fixture_store(root)[0]
            descriptor = model_store.catalog_models()[0].descriptor
            model = root / record.path
            (model / "redactio-model.json").write_text(
                json.dumps(
                    {
                        "name": descriptor.name,
                        "version": descriptor.version,
                        "repository": descriptor.repository,
                    }
                )
            )
            (root / "manifest.json").write_text(
                json.dumps(
                    {
                        "models": [
                            {
                                "name": descriptor.name,
                                "version": descriptor.version,
                                "path": record.path,
                            }
                        ]
                    }
                )
            )
            before = {path: path.read_bytes() for path in root.rglob("*") if path.is_file()}
            engine = Engine(root)
            try:
                for path in before:
                    path.chmod(0o444)
                with patch.object(biomedbert, "_load_pipeline", return_value=lambda _: []):
                    configure(engine, descriptor.name)
                with self.assertRaisesRegex(EngineError, "model_in_use"):
                    with _native_lock(
                        model / "redactio-model.json", create=False, code="model_in_use"
                    ):
                        self.fail("legacy model lease missing")
                self.assertEqual(
                    {path: path.read_bytes() for path in root.rglob("*") if path.is_file()},
                    before,
                )
            finally:
                del engine
                for path in before:
                    path.chmod(0o600)

    def test_discovery_and_failed_construction_leave_no_lease(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            records = fixture_store(root)
            engine = Engine(root)
            self.assertTrue(all(model.compatible for model in engine.available_models()))
            self.assertEqual(removal(root, records[1].descriptor.name)["type"], "result")
            with patch.object(biomedbert, "_load_pipeline", side_effect=ValueError("synthetic")):
                with self.assertRaises(EngineError):
                    configure(engine, records[0].descriptor.name)
            self.assertEqual(removal(root, records[0].descriptor.name)["type"], "result")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--worker":
        root = Path(sys.argv[2])
        engine = Engine(root)
        with patch.object(biomedbert, "_load_pipeline", return_value=lambda _: []):
            for record in model_store.read_registry(root).models:
                configure(engine, record.descriptor.name)
        marker = root / "worker-ready.tmp"
        marker.write_text(json.dumps({"pid": os.getpid()}))
        marker.replace(root / "worker-ready.json")
        time.sleep(120)
    else:
        unittest.main()
