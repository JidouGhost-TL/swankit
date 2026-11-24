"""Admission checks for retained save edit inputs and their recorded outcomes."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from stage_save_edits import stage_save_edits


class SaveEditStaging(unittest.TestCase):
    def fixture(self, root):
        source = root / "input.eeprom"
        source.write_bytes(bytes(range(128)))
        manifest = root / "outcomes.json"
        records = [
            {"input_path": str(source), "input_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
             "definition": "synthetic", "name": "save.eeprom", "field": 0, "value": "7",
             "options": [], "expect": {"outcome": "written", "output_sha256": "ab" * 32},
             "oracle": "recorded edit receipt"},
            {"input_path": str(source), "input_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
             "definition": "synthetic", "name": "save.eeprom", "field": 0, "value": "[1, 2]",
             "options": ["--no-repair"], "expect": {"outcome": "refused", "reason": "input-unusable"},
             "oracle": "definition validity program rejects the input"},
        ]
        manifest.write_text(json.dumps(records))
        return manifest, records

    def test_enrollment_freezes_expectations_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, _ = self.fixture(root)
            other = root / "ws-rom/spec.json"
            other.parent.mkdir()
            other.write_bytes(b"frozen unrelated selection")
            self.assertEqual(stage_save_edits(manifest, root), 2)
            spec = json.loads((root / "swankit-save-edits/spec.json").read_text())
            self.assertEqual(spec["format"], "swankit-save-edits")
            second = spec["samples"]["sample-000001.dat"]
            self.assertEqual(second["args"]["options"], ["--no-repair"])
            self.assertEqual(second["expect"]["reason"], "input-unusable")
            self.assertTrue((root / "swankit-save-edits/sample-000000.dat").is_symlink())
            frozen = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
            with self.assertRaisesRegex(ValueError, "frozen"):
                stage_save_edits(manifest, root)
            self.assertEqual(frozen, {p: p.read_bytes() for p in root.rglob("*") if p.is_file()})

    def test_changed_input_creates_no_selection(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, records = self.fixture(root)
            Path(records[0]["input_path"]).write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "differ"):
                stage_save_edits(manifest, root)
            self.assertFalse((root / "swankit-save-edits").exists())

    def test_invalid_contract_creates_no_selection(self):
        cases = (("field", -1), ("field", True), ("value", "not json"), ("options", ["--force"]),
                 ("options", ["--no-repair", "--no-repair"]), ("name", "dir/save.eeprom"),
                 ("oracle", ""), ("definition", ""),
                 ("expect", {"outcome": "refused", "reason": "something"}),
                 ("expect", {"outcome": "written"}),
                 ("expect", {"outcome": "written", "output_sha256": "AB" * 32}))
        for field, value in cases:
            with self.subTest(field=field, value=value), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                manifest, records = self.fixture(root)
                records[0][field] = value
                manifest.write_text(json.dumps(records))
                with self.assertRaises(ValueError):
                    stage_save_edits(manifest, root)
                self.assertFalse((root / "swankit-save-edits").exists())


if __name__ == "__main__":
    unittest.main()
