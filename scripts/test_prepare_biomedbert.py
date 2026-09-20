"""Synthetic build-time model preparation checks; no model download."""

import hashlib
import importlib.util
import json
import tempfile
import unittest
from io import BytesIO
from pathlib import Path
from unittest.mock import patch


class ModelPreparationTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "prepare", Path(__file__).with_name("prepare-biomedbert.py")
        )
        self.prepare = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.prepare)
        self.payload = b"synthetic model bytes"
        self.inputs = {
            "name": "test-model",
            "repository": "test/model",
            "revision": "a" * 40,
            "files": {"model.safetensors": hashlib.sha256(self.payload).hexdigest()},
        }

    def test_verified_install_preserves_other_models_and_is_offline_when_repeated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            previous = {"name": "another-supported-model", "version": "1", "path": "other"}
            retired = {"name": "de_core_news_lg", "version": "3.8.0", "path": "lg"}
            (root / "lg").mkdir()
            (root / "lg/retained").write_bytes(b"previous model")
            (root / "manifest.json").write_text(json.dumps({"models": [previous, retired]}))
            with patch.object(self.prepare, "urlopen", return_value=BytesIO(self.payload)):
                self.prepare.prepare(root, self.inputs)
            manifest = json.loads((root / "manifest.json").read_text())
            self.assertEqual(
                manifest["models"],
                [
                    previous,
                    {
                        "name": "test-model",
                        "version": "a" * 40,
                        "path": "biomedbert-de",
                    },
                ],
            )
            self.assertEqual((root / "biomedbert-de/model.safetensors").read_bytes(), self.payload)
            self.assertEqual((root / "lg/retained").read_bytes(), b"previous model")
            self.assertEqual(
                json.loads((root / "biomedbert-de/redactio-model.json").read_text()),
                {
                    "name": "test-model",
                    "version": "a" * 40,
                    "repository": "test/model",
                },
            )
            with patch.object(self.prepare, "urlopen", side_effect=AssertionError("network")):
                self.prepare.prepare(root, self.inputs)
            self.assertEqual(json.loads((root / "manifest.json").read_text()), manifest)

    def test_bad_download_leaves_existing_manifest_and_no_advertised_model(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            before = b'{"models": []}\n'
            (root / "manifest.json").write_bytes(before)
            with patch.object(self.prepare, "urlopen", return_value=BytesIO(b"corrupt")):
                with self.assertRaisesRegex(ValueError, "checksum"):
                    self.prepare.prepare(root, self.inputs)
            self.assertEqual((root / "manifest.json").read_bytes(), before)
            self.assertFalse((root / "biomedbert-de").exists())

    def test_alternative_install_keeps_both_models_and_is_repeatable_offline(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            alternative = {**self.inputs, "name": "alternative", "directory": "alternative-de"}
            with patch.object(
                self.prepare, "urlopen", side_effect=lambda *a, **k: BytesIO(self.payload)
            ):
                self.prepare.prepare(root, self.inputs)
                self.prepare.prepare(root, alternative)
            entries = json.loads((root / "manifest.json").read_text())["models"]
            self.assertEqual(
                [entry["path"] for entry in entries], ["biomedbert-de", "alternative-de"]
            )
            with patch.object(self.prepare, "urlopen", side_effect=AssertionError("network")):
                self.prepare.prepare(root, alternative)
                self.prepare.prepare(root, self.inputs)
            self.assertEqual(json.loads((root / "manifest.json").read_text())["models"], entries)

    def test_model_directory_cannot_escape_setup_root(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ("../escape", "/absolute", "..\\escape", "C:\\outside"):
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, "directory"):
                    self.prepare.prepare(Path(directory), {**self.inputs, "directory": name})


if __name__ == "__main__":
    unittest.main()
