"""Text enrollment requires independent references and preserves frozen specs."""
import hashlib
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "stage-corpus.py"
spec = importlib.util.spec_from_file_location("stage_text_tests", SCRIPT)
staging = importlib.util.module_from_spec(spec)
spec.loader.exec_module(staging)


class TextStagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "staging"
        self.output.mkdir()
        self.frozen = self.output / "versions.json"
        self.frozen.write_text('{"existing":"frozen"}\n')
        rom_spec = self.output / "ws-rom" / "spec.json"
        rom_spec.parent.mkdir()
        rom_spec.write_text("frozen cartridge expectation\n")
        source = self.root / "input"
        source.write_bytes(bytes([1, 255]))
        self.oracle = self.root / "oracle.txt"
        self.oracle.write_text("independent synthetic text\n")
        self.record = {
            "name": "synthetic.text", "path": str(source),
            "args": {"definition": "synthetic", "strings": [], "fonts": []},
            "expect": {"sha256": hashlib.sha256(source.read_bytes()).hexdigest()},
            "references": [{"path": str(self.oracle),
                            "sha256": hashlib.sha256(self.oracle.read_bytes()).hexdigest()}],
        }
        self.manifest = self.root / "manifest.json"
        self.args = SimpleNamespace(output=self.output, text_manifest=self.manifest,
                                    text_spec_dir="ws-text", progress_log=None)

    def write_manifest(self, records=None):
        self.manifest.write_text(json.dumps({"samples": [self.record] if records is None else records,
                                             "provenance": {"method": "synthetic independent oracle"}}))

    def snapshot(self):
        return {p.relative_to(self.output): p.read_bytes()
                for p in self.output.rglob("*") if p.is_file()}

    def test_enrollment_keeps_versions_and_other_specs_and_refuses_replacement(self):
        original = self.snapshot()
        self.write_manifest()
        staging.stage_text(self.args)
        for name, data in original.items():
            self.assertEqual((self.output / name).read_bytes(), data)
        spec = json.loads((self.output / "ws-text" / "spec.json").read_text())
        self.assertEqual(spec["samples"][self.record["name"]]["expect"], self.record["expect"])
        self.assertTrue((self.output / "ws-text" / self.record["name"]).is_symlink())
        frozen = self.snapshot()
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            staging.stage_text(self.args)
        self.assertEqual(self.snapshot(), frozen)
        self.args.text_spec_dir = "ws-text-extra"
        staging.stage_text(self.args)
        for name, data in frozen.items():
            self.assertEqual((self.output / name).read_bytes(), data)

    def test_changed_source_or_oracle_is_rejected_before_any_namespace_write(self):
        original = self.snapshot()
        self.record["expect"]["sha256"] = "0" * 64
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, "source differs"):
            staging.stage_text(self.args)
        self.assertEqual(self.snapshot(), original)
        self.record["expect"]["sha256"] = hashlib.sha256(Path(self.record["path"]).read_bytes()).hexdigest()
        self.write_manifest()
        self.oracle.write_text("changed independent reference\n")
        with self.assertRaisesRegex(ValueError, "oracle source differs"):
            staging.stage_text(self.args)
        self.assertEqual(self.snapshot(), original)

    def test_missing_provenance_empty_or_duplicate_samples_are_rejected(self):
        original = self.snapshot()
        for records in [[], [self.record, self.record]]:
            self.write_manifest(records)
            with self.assertRaises(ValueError):
                staging.stage_text(self.args)
            self.assertEqual(self.snapshot(), original)
        self.record["references"] = []
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, "independent oracle provenance"):
            staging.stage_text(self.args)
        self.assertEqual(self.snapshot(), original)

    def test_names_cannot_escape_the_namespace(self):
        original = self.snapshot()
        for name in ["../escape.text", ".hidden.text", "wrong.raw"]:
            self.record["name"] = name
            self.write_manifest()
            with self.assertRaisesRegex(ValueError, "invalid text sample name"):
                staging.stage_text(self.args)
            self.assertEqual(self.snapshot(), original)

    def test_malformed_metadata_is_rejected_before_writes(self):
        original = self.snapshot()
        for args in [None, {"strings": None}, {"fonts": None},
                     {"fonts": [{"count": -1}]}, {"fonts": [{"count": True}]}]:
            self.record["args"] = args
            self.write_manifest()
            with self.assertRaises(ValueError):
                staging.stage_text(self.args)
            self.assertEqual(self.snapshot(), original)
        self.record["args"] = {}
        self.write_manifest()
        self.args.text_spec_dir = "../invalid"
        with self.assertRaisesRegex(ValueError, "invalid text spec directory"):
            staging.stage_text(self.args)
        self.assertEqual(self.snapshot(), original)


if __name__ == "__main__":
    unittest.main()
