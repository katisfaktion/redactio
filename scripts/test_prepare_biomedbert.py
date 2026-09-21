"""The setup command delegates to the same checked catalog installer as the app."""

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from uuid import UUID

from redactio_sidecar import model_manager
from redactio_sidecar.ipc import EngineError
from redactio_sidecar.model_store import catalog_models


class PreparationTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "prepare", Path(__file__).with_name("prepare-biomedbert.py")
        )
        self.prepare = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.prepare)

    def test_both_choices_pass_pinned_catalog_descriptor_and_generated_job(self):
        for entry in catalog_models():

            def install(root, descriptor, job_id, emit, expected=entry.descriptor):
                self.assertEqual(descriptor, expected)
                self.assertEqual(UUID(job_id).version, 4)
                self.assertTrue(callable(emit))
                return descriptor

            with tempfile.TemporaryDirectory() as temporary:
                with patch.object(
                    model_manager, "install_model", side_effect=install, create=True
                ):
                    result = self.prepare.prepare(Path(temporary), entry.key)
                self.assertEqual(result, entry.descriptor)
                self.assertEqual(list(Path(temporary).iterdir()), [])

    def test_worker_failure_is_not_reported_as_prepared(self):
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(
                model_manager,
                "install_model",
                create=True,
                side_effect=EngineError("model_hash_mismatch"),
            ),
            self.assertRaisesRegex(EngineError, "model_hash_mismatch"),
        ):
            self.prepare.prepare(Path(temporary), "biomedbert")

    def test_invalid_catalog_key_fails_without_install(self):
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch.object(
                model_manager,
                "install_model",
                create=True,
                side_effect=AssertionError("installer called"),
            ),
            self.assertRaises(ValueError),
        ):
            self.prepare.prepare(Path(temporary), "unknown")


if __name__ == "__main__":
    unittest.main()
