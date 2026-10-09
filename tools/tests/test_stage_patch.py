"""Patch enrollment preserves frozen selections and requires independent evidence."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


TOOLS = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("stage_patch", TOOLS / "stage_patch.py")
STAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGE)


class PatchStagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.output = root / "staging"
        existing = self.output / "existing"
        existing.mkdir(parents=True)
        (existing / "spec.json").write_text('{"samples":{"fixture":{"expect":"frozen"}}}')
        (self.output / "versions.json").write_text('{"existing":"frozen"}')
        self.source = root / "source.ips"
        self.base = root / "base.fixture"
        self.target = root / "target.fixture"
        self.reference = root / "evidence.txt"
        self.source.write_bytes(b"PATCH\0\0\1\0\1xEOF")
        self.base.write_bytes(b"abc")
        self.target.write_bytes(b"axc")
        self.reference.write_text("independent synthetic patch evidence")
        self.row = {"name": "sample.ips", "source": str(self.source), "base": str(self.base),
                    "target": str(self.target), "expect": {
                        "sha256": self.digest(self.source), "base_sha256": self.digest(self.base),
                        "patched_sha256": self.digest(self.target), "base_size": 3,
                        "patched_size": 3, "records": 1},
                    "provenance": {"reference_kind": "published-digest", "references": [
                        {"path": str(self.reference), "sha256": self.digest(self.reference)}]}}
        self.manifest = root / "manifest.json"

    def digest(self, path):
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def frozen(self):
        return {p.relative_to(self.output): p.read_bytes() for p in self.output.rglob("*") if p.is_file()}

    def enroll(self, rows=None):
        self.manifest.write_text(json.dumps({"samples": rows if rows is not None else [self.row]}))
        return STAGE.stage_patch(self.manifest, self.output)

    def test_enrollment_preserves_existing_specs_and_refuses_overwrite(self):
        original = self.frozen()
        self.assertEqual(self.enroll(), 1)
        frozen = self.frozen()
        for path, value in original.items():
            self.assertEqual(frozen[path], value)
        spec = json.loads((self.output / "ws-patch/spec.json").read_text())
        sample = spec["samples"]["sample.ips"]
        self.assertEqual((self.output / "ws-patch" / sample["args"]["base"]).read_bytes(), b"abc")
        self.assertEqual((self.output / "ws-patch" / sample["args"]["target"]).read_bytes(), b"axc")
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            self.enroll()
        self.assertEqual(self.frozen(), frozen)

    def test_each_independent_digest_size_and_reference_must_match_before_writes(self):
        original = copy.deepcopy(self.row)
        for key in ("sha256", "base_sha256", "patched_sha256", "base_size", "patched_size"):
            with self.subTest(key=key):
                self.row = copy.deepcopy(original)
                self.row["expect"][key] = 9 if key.endswith("size") else "0" * 64
                with self.assertRaises(ValueError): self.enroll()
                self.assertFalse((self.output / "ws-patch").exists())
        self.row = copy.deepcopy(original)
        self.row["provenance"]["references"][0]["sha256"] = "0" * 64
        with self.assertRaises(ValueError): self.enroll()
        self.assertFalse((self.output / "ws-patch").exists())

    def test_names_duplicates_empty_and_missing_evidence_are_rejected(self):
        for name in ("../sample.ips", "/sample.ips", ".hidden.ips", "spec.json", ""):
            self.row["name"] = name
            with self.assertRaises(ValueError): self.enroll()
        self.row["name"] = "sample.ips"
        with self.assertRaises(ValueError): self.enroll([self.row, self.row])
        with self.assertRaises(ValueError): self.enroll([])
        self.row["provenance"]["references"] = []
        with self.assertRaises(ValueError): self.enroll()
        self.assertFalse((self.output / "ws-patch").exists())

    def test_same_patch_digest_can_retain_separate_provenance(self):
        duplicate = copy.deepcopy(self.row)
        duplicate["name"] = "other.ips"
        self.assertEqual(self.enroll([self.row, duplicate]), 2)
        spec = json.loads((self.output / "ws-patch/spec.json").read_text())
        self.assertEqual(len(spec["samples"]), 2)

    def test_cli_patch_selection_is_mutually_exclusive(self):
        self.manifest.write_text(json.dumps({"samples": [self.row]}))
        result = subprocess.run([sys.executable, str(TOOLS / "stage-corpus.py"),
                                 "--patch-manifest", str(self.manifest), "--titles", str(self.output),
                                 "--output", str(self.output)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertFalse((self.output / "ws-patch").exists())
