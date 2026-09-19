"""Run with the sidecar environment: python scripts/test_packaging.py."""

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from docx import Document

ROOT = Path(__file__).resolve().parents[1]


class PackagingTests(unittest.TestCase):
    def test_canary_generation_refuses_to_overwrite_and_contains_contact(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "corpus"
            command = [
                sys.executable,
                str(ROOT / "scripts/generate-corpus.py"),
                "--output",
                str(output),
                "--count",
                "1",
            ]
            result = subprocess.run(command, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            source = output / "case-0001.docx"
            self.assertIn(
                "anna.beispiel@example.invalid",
                "\n".join(p.text for p in Document(source).paragraphs),
            )
            before = source.read_bytes()
            self.assertNotEqual(
                subprocess.run(command, capture_output=True, check=False).returncode, 0
            )
            self.assertEqual(source.read_bytes(), before)
            manifest = json.loads((output / "expectations.json").read_text())
            self.assertEqual(manifest["documents"], 1)

    def test_model_wheel_staging_rejects_bad_hash_and_writes_local_manifest(self):
        spec = importlib.util.spec_from_file_location("stage", ROOT / "packaging/stage.py")
        self.assertTrue(Path(spec.origin).is_file(), "model staging implementation missing")
        stage = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(stage)
        import hashlib
        import zipfile

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wheel = root / "model.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr(
                    "de_core_news_lg/de_core_news_lg-3.8.0/meta.json",
                    '{"lang":"de","name":"core_news_lg","version":"3.8.0"}',
                )
                archive.writestr("de_core_news_lg/de_core_news_lg-3.8.0/config.cfg", "[nlp]")
            inputs = {"name": "de_core_news_lg", "version": "3.8.0", "sha256": "0" * 64}
            with self.assertRaises(ValueError):
                stage.stage_model(wheel, root / "models", inputs)
            self.assertFalse((root / "models").exists())
            inputs["sha256"] = hashlib.sha256(wheel.read_bytes()).hexdigest()
            stage.stage_model(wheel, root / "models", inputs)
            manifest = json.loads((root / "models/manifest.json").read_text())
            self.assertEqual(
                manifest,
                {
                    "models": [
                        {"name": "de_core_news_lg", "version": "3.8.0", "path": "de_core_news_lg"}
                    ]
                },
            )
            self.assertTrue((root / "models/de_core_news_lg/config.cfg").is_file())


if __name__ == "__main__":
    unittest.main()
