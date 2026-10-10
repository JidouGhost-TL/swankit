"""Independent-oracle admission and preservation of frozen selections."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from stage_interop import stage_interop


class InteropStaging(unittest.TestCase):
    def fixture(self, root):
        input_path = root / "payload.dat"
        input_path.write_bytes(bytes(544))
        manifest = root / "oracle.json"
        record = {
            "input_path": str(input_path),
            "input_sha256": hashlib.sha256(input_path.read_bytes()).hexdigest(),
            "operation": 2, "output_bytes": 256,
            "output_sha256": hashlib.sha256(bytes(256)).hexdigest(),
            "oracle": "independent synthetic Java image",
        }
        manifest.write_text(json.dumps([record]))
        return manifest, record

    def test_refuses_overwrite_without_changing_frozen_bytes(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, record = self.fixture(root)
            stage_interop(manifest, root)
            spec = root / "swankit-interop/spec.json"
            frozen = spec.read_bytes()
            record["output_sha256"] = "1" * 64
            manifest.write_text(json.dumps([record]))
            with self.assertRaisesRegex(ValueError, "frozen"):
                stage_interop(manifest, root)
            self.assertEqual(spec.read_bytes(), frozen)

    def test_rejects_mutated_source_before_creating_selection(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, record = self.fixture(root)
            Path(record["input_path"]).write_bytes(b"changed source")
            with self.assertRaisesRegex(ValueError, "differs"):
                stage_interop(manifest, root)
            self.assertFalse((root / "swankit-interop").exists())

    def test_invalid_contract_metadata_creates_no_selection(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, record = self.fixture(root)
            for field, value in (("operation", True), ("output_bytes", True),
                                 ("output_bytes", 257), ("oracle", "")):
                changed = dict(record)
                changed[field] = value
                manifest.write_text(json.dumps([changed]))
                with self.assertRaises(ValueError):
                    stage_interop(manifest, root)
                self.assertFalse((root / "swankit-interop").exists())


if __name__ == "__main__":
    unittest.main()
