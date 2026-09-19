"""Run with the sidecar environment: python scripts/test_packaging.py."""

import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from docx import Document

ROOT = Path(__file__).resolve().parents[1]


class PackagingTests(unittest.TestCase):
    def test_corpus_is_reproducible_and_edges_are_separate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            command = [sys.executable, str(ROOT / "scripts/generate-corpus.py")]
            result = subprocess.run(
                command
                + [
                    "--output",
                    str(root / "first"),
                    "--count",
                    "400",
                    "--edge-output",
                    str(root / "edges"),
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(list((root / "first").glob("*.docx"))), 400)
            manifest = json.loads((root / "first/expectations.json").read_text())
            self.assertEqual(len(manifest["files"]), 400)
            self.assertNotIn("@", json.dumps(manifest))
            for entry in manifest["files"]:
                self.assertEqual(
                    hashlib.sha256((root / "first" / entry["path"]).read_bytes()).hexdigest(),
                    entry["sha256"],
                )
            result = subprocess.run(
                command + ["--output", str(root / "second"), "--count", "1"],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                (root / "first/case-0001.docx").read_bytes(),
                (root / "second/case-0001.docx").read_bytes(),
            )
            edges = json.loads((root / "edges/expectations.json").read_text())
            self.assertEqual(
                {entry["profile"] for entry in edges["files"]},
                {"warnings", "corrupt", "empty", "repeated", "unicode", "tamper"},
            )
            from redactio_sidecar.extract import extract_document
            from redactio_sidecar.ipc import EngineError

            for entry in edges["files"]:
                source = root / "edges" / entry["path"]
                if entry["profile"] == "corrupt":
                    with self.assertRaisesRegex(EngineError, "invalid_docx"):
                        extract_document(source)
                else:
                    self.assertEqual(extract_document(source).warnings, entry["warnings"])
            self.assertEqual(extract_document(root / "edges/empty.docx").text, "")
            self.assertEqual(
                extract_document(root / "edges/repeated.docx").text.count("Max Mustermann"), 3
            )
            self.assertIn("🙂 e\u0301", extract_document(root / "edges/unicode.docx").text)

    def test_invalid_generation_preserves_user_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            command = [sys.executable, str(ROOT / "scripts/generate-corpus.py"), "--output"]
            for count in ("0", "-1"):
                self.assertNotEqual(
                    subprocess.run(
                        command + [str(root / "absent"), "--count", count], capture_output=True
                    ).returncode,
                    0,
                )
                self.assertFalse((root / "absent").exists())
            user = root / "user.docx"
            user.write_bytes(b"user bytes")
            self.assertNotEqual(
                subprocess.run(
                    command + [str(root), "--count", "1"], capture_output=True
                ).returncode,
                0,
            )
            self.assertEqual(user.read_bytes(), b"user bytes")
            self.assertEqual(len(list(root.iterdir())), 1)

    @unittest.skipUnless(sys.platform == "win32", "requires Windows command resolution")
    def test_desktop_notices_resolves_pnpm_cmd_from_path(self):
        spec = importlib.util.spec_from_file_location("notices", ROOT / "packaging/notices.py")
        notices = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(notices)
        with tempfile.TemporaryDirectory(prefix="redactio notices ") as directory:
            root = Path(directory)
            (root / "packaging").mkdir()
            (root / "packaging/build-inputs.json").write_text('{"notices": []}')
            (root / "LICENSE").write_text("Synthetic application license")
            metadata = root / "cargo-metadata.json"
            metadata.write_text('{"packages": []}')
            dependency = root / "dependency"
            dependency.mkdir()
            (dependency / "package.json").write_text('{"version": "1.0.0"}')
            (dependency / "LICENSE").write_text("Synthetic dependency license")
            command_dir = root / "command directory"
            command_dir.mkdir()
            (command_dir / "licenses.json").write_text(
                json.dumps(
                    {"MIT": [{"name": "fixture", "license": "MIT", "paths": [str(dependency)]}]}
                )
            )
            (command_dir / "pnpm.cmd").write_text(
                '@echo off\nif not "%*"=="licenses list --prod --json" exit /b 9\n'
                'type "%~dp0licenses.json"\n'
            )
            with patch.dict(os.environ, {"PATH": str(command_dir), "PATHEXT": ".CMD"}):
                output = notices.desktop_notices(root, metadata)
            self.assertIn("fixture 1.0.0\nLicense: MIT\nSynthetic dependency license", output)

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
