"""Frozen save enrollment validates independent bytes before creating files."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

spec = importlib.util.spec_from_file_location("save_staging", Path(__file__).resolve().parents[1] / "stage-corpus.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SaveStagingTests(unittest.TestCase):
    def fixture(self, root):
        image = root / "source.eeprom"
        image.write_bytes(bytes([1, 2, 3, 4]))
        record = {"id": "synthetic", "path": str(image), "name": "source.eeprom",
                  "decoded": {"valid": True}, "sha256": hashlib.sha256(image.read_bytes()).hexdigest()}
        manifest = root / "manifest.json"
        manifest.write_text(json.dumps({"records": [record], "normalization": "none"}))
        return SimpleNamespace(save_manifest=manifest, save_namespace="ws-saves-parity",
                               output=root / "staging", progress_log=None), record

    def test_namespace_refuses_overwrite_and_preserves_frozen_spec(self):
        with tempfile.TemporaryDirectory(prefix="save-staging-") as temporary:
            args, _ = self.fixture(Path(temporary))
            module.stage_saves(args)
            frozen = args.output / args.save_namespace / "spec.json"
            before = frozen.read_bytes()
            with self.assertRaisesRegex(ValueError, "frozen expectations"):
                module.stage_saves(args)
            self.assertEqual(frozen.read_bytes(), before)

    def test_bad_independent_digest_creates_no_partial_namespace(self):
        with tempfile.TemporaryDirectory(prefix="save-staging-") as temporary:
            args, record = self.fixture(Path(temporary))
            bad = {**record, "sha256": "0" * 64}
            args.save_manifest.write_text(json.dumps({"records": [record, bad], "normalization": "none"}))
            with self.assertRaisesRegex(ValueError, "independent oracle"):
                module.stage_saves(args)
            self.assertFalse(args.output.exists())

    def test_identical_images_keep_distinct_source_and_companion_context(self):
        with tempfile.TemporaryDirectory(prefix="save-staging-") as temporary:
            args, record = self.fixture(Path(temporary))
            other = {**record, "name": "other.eeprom", "context": {"lookup": [5]}}
            args.save_manifest.write_text(json.dumps({"records": [record, other], "normalization": "none"}))
            module.stage_saves(args)
            samples = json.loads((args.output / args.save_namespace / "spec.json").read_text())["samples"]
            self.assertEqual({s["args"]["name"] for s in samples.values()}, {"source.eeprom", "other.eeprom"})
            self.assertIn({"lookup": [5]}, [s["args"]["context"] for s in samples.values()])


if __name__ == "__main__":
    unittest.main()
