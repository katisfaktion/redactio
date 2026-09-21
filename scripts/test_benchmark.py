"""Run with stdlib Python; all documents and protocol peers are synthetic."""

import copy
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ENGINE = {
    "engine_version": "redactio-sidecar 0.1.0",
    "model_name": "OpenMed-PII-German-BiomedBERT-Large-340M-v1",
    "model_version": "ce797d58600cc20bba9a2500dafc0b7f5c3270c1",
    "extraction_version": "1",
    "recognizers": ["SyntheticRecognizer"],
}


def fake(mode):
    # A real pipe peer catches framing, identity, blocked I/O and cleanup mistakes.
    if mode == "high_memory":
        allocation = bytearray(64 * 1024 * 1024)
        for index in range(0, len(allocation), 4096):
            allocation[index] = 1  # Commit resident pages on Windows too.
    for line in sys.stdin:
        request = json.loads(line)
        payload = request["payload"]
        kind = request["type"]
        if mode == "stall":
            time.sleep(60)
        if mode == "overflow":
            sys.stdout.write("x" * 4097)
            sys.stdout.flush()
            time.sleep(60)
        if mode == "partial":
            sys.stdout.write('{"id":')
            sys.stdout.flush()
            return
        if kind == "ping":
            result = {"protocol_version": 1}
        elif kind == "configure":
            assert payload["config"]["custom_rules"] == []
            assert payload["config"]["model"] == ENGINE["model_name"]
            assert payload["config"]["model_entities"] == ["FIRSTNAME", "FUTURE_TYPE", "ZIPCODE"]
            result = {key: payload[key] for key in ("sync_pair_id", "processing_revision")}
            result["engine"] = ENGINE
        else:
            result = {key: value for key, value in payload.items() if key != "source_path"}
            result["redacted_at"] = result["redacted_at"].replace("+00:00", "Z")
            result.update(
                markdown="private-output",
                body="private-output",
                original_text="ä🙂x",
                detections=[],
                redactions=[],
                warnings=[],
                body_was_empty=False,
                review_status="pending",
                engine=ENGINE,
            )
            if mode == "safe_error" and payload["doc_id"] == "doc-0002":
                kind = "error"
                result = {"code": "invalid_docx", "retryable": False}
            elif mode == "missing":
                del result["original_text"]
            elif mode == "wrong_identity":
                result["source_hash_sha256"] = "0" * 64
            elif mode == "wrong_engine":
                result["engine"] = {**ENGINE, "model_version": "9.0"}
            elif mode == "bad_error":
                kind = "error"
                result = {"code": "secret path and text", "retryable": "false"}
        reply = {"id": request["id"], "type": kind + "_result", "payload": result}
        if kind == "error":
            reply["type"] = "error"
        if mode == "wrong_id":
            reply["id"] = "unrelated"
        if mode == "empty":
            reply["payload"] = {}
        sys.stderr.write("PRIVATE STDERR MUST NOT ESCAPE\n")
        print(json.dumps(reply), flush=True)


class BenchmarkTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import benchmark

        cls.b = benchmark

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.corpus = self.root / "private-corpus"
        self.corpus.mkdir()
        (self.corpus / "private-name.docx").write_bytes(b"synthetic-one")
        (self.corpus / "other.docx").write_bytes(b"two")
        self.models = self.root / "models"
        self.models.mkdir()
        (self.models / "manifest.json").write_text(
            json.dumps(
                {
                    "models": [
                        {
                            "name": ENGINE["model_name"],
                            "version": ENGINE["model_version"],
                            "path": "biomedbert-de",
                        }
                    ]
                }
            )
        )
        (self.models / "biomedbert-de").mkdir()
        (self.models / "biomedbert-de/config.json").write_text(
            json.dumps(
                {
                    "id2label": {
                        "0": "O",
                        "1": "B-FIRSTNAME",
                        "2": "I-FIRSTNAME",
                        "3": "B-ZIPCODE",
                        "4": "B-FUTURE_TYPE",
                    }
                }
            )
        )

    def command(self, mode="ok"):
        # Windows venv python.exe is a launcher: measure/kill the actual peer.
        return [sys._base_executable, str(Path(__file__).resolve()), "--fake", mode]

    def test_metrics_account_for_real_child_and_unicode_without_content(self):
        before = {path: path.read_bytes() for path in self.corpus.iterdir()}
        result = self.b.run_benchmark(self.command(), self.models, self.corpus)
        self.b.validate_metrics(result)
        self.assertEqual((result["documents"], result["processed"], result["failed"]), (2, 2, 0))
        self.assertEqual(
            result["input_bytes"], {"count": 2, "min": 3, "max": 13, "total": 16, "median": 8.0}
        )
        self.assertEqual(
            result["characters"], {"count": 2, "min": 3, "max": 3, "total": 6, "median": 3.0}
        )
        self.assertGreater(result["machine"]["ram_bytes"], 0)
        self.assertGreater(result["machine"]["logical_cpus"], 0)
        self.assertTrue(result["machine"]["cpu"])
        encoded = json.dumps(result)
        for private in [str(self.root), "private-name", "private-output", "ä🙂x", "PRIVATE STDERR"]:
            self.assertNotIn(private, encoded)
        self.assertEqual(before, {path: path.read_bytes() for path in self.corpus.iterdir()})
        for field, value in [
            ("documents", 0),
            ("processed", 0),
            ("peak_memory_bytes", 0),
            ("total_seconds", -1),
            ("cold_start_seconds", float("nan")),
            ("text", "private"),
        ]:
            bad = copy.deepcopy(result)
            bad[field] = value
            with self.subTest(field=field), self.assertRaises((ValueError, AssertionError)):
                self.b.validate_metrics(bad)

    def test_safe_document_error_is_accounted_but_never_positive_throughput(self):
        result = self.b.run_benchmark(self.command("safe_error"), self.models, self.corpus)
        self.assertEqual((result["processed"], result["failed"]), (1, 1))
        self.assertEqual(result["characters"]["count"], 1)
        self.assertIsNone(result["documents_per_minute"])
        self.b.validate_metrics(result)

    def test_peak_memory_belongs_to_this_child_not_earlier_children(self):
        high = self.b.run_benchmark(self.command("high_memory"), self.models, self.corpus)
        low = self.b.run_benchmark(self.command(), self.models, self.corpus)
        self.assertGreater(high["peak_memory_bytes"] - low["peak_memory_bytes"], 32 * 1024 * 1024)

    def test_rejects_untrustworthy_responses_and_cleans_up(self):
        for mode in [
            "wrong_id",
            "empty",
            "missing",
            "wrong_identity",
            "wrong_engine",
            "bad_error",
            "partial",
        ]:
            with self.subTest(mode=mode), self.assertRaises(self.b.BenchmarkError):
                self.b.run_benchmark(self.command(mode), self.models, self.corpus)
        for mode in ["stall", "overflow"]:
            start = time.perf_counter()
            child = self.b.Child(self.command(mode), os.environ.copy(), self.root)
            with self.assertRaises(self.b.BenchmarkError):
                with child:
                    child.request("ping", {}, time.perf_counter() + 0.15, limit=4096)
            self.assertIsNotNone(child.process.returncode)
            self.assertLess(time.perf_counter() - start, 5)

    def test_empty_corpus_and_output_overwrite_are_rejected(self):
        empty = self.root / "empty"
        empty.mkdir()
        with self.assertRaises(self.b.BenchmarkError):
            self.b.run_benchmark(self.command(), self.models, empty)
        output = self.root / "existing.json"
        output.write_bytes(b"preserve me")
        completed = subprocess.run(
            [
                sys.executable,
                str(Path(self.b.__file__)),
                "--package",
                str(self.root),
                "--corpus",
                str(self.corpus),
                "--output",
                str(output),
            ],
            capture_output=True,
            timeout=10,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(output.read_bytes(), b"preserve me")
        self.assertNotIn(str(self.root).encode(), completed.stderr)
        self.assertNotIn(b"Traceback", completed.stderr)

    @unittest.skipIf(sys.platform == "win32", "Windows symlink creation requires host privilege")
    def test_redirected_corpus_subdirectory_is_not_silently_omitted(self):
        (self.corpus / "redirected").symlink_to(self.models, target_is_directory=True)
        with self.assertRaises(self.b.BenchmarkError):
            self.b.run_benchmark(self.command(), self.models, self.corpus)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--fake":
        fake(sys.argv[2])
    else:
        unittest.main()
