"""Graphics enrollment must preserve existing frozen cartridge expectations."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest


MODULE_PATH = Path(__file__).resolve().parents[1] / "stage-corpus.py"
SPEC = importlib.util.spec_from_file_location("stage_corpus", MODULE_PATH)
STAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGE)


class GraphicsStagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.output = root / "staging"
        self.output.mkdir()
        self.original = {"existing": {"format": "ws-rom", "expectation": "frozen"}}
        (self.output / "versions.json").write_text(json.dumps(self.original))
        self.source = root / "sample.tiles"
        self.source.write_bytes(bytes(16))
        self.record = {
            "name": "sample.tiles", "path": str(self.source),
            "args": {"operation": "tiles", "tile_format": "2bpp"},
            "expect": {"sha256": hashlib.sha256(self.source.read_bytes()).hexdigest()},
            "reference_sha256": "a" * 64,
        }
        self.manifest = root / "manifest.json"

    def enroll(self):
        self.manifest.write_text(json.dumps({"samples": [self.record]}))
        STAGE.stage_graphics(SimpleNamespace(output=self.output, gfx_manifest=self.manifest, progress_log=None))

    def test_addition_preserves_ledger_and_refuses_reenrollment(self):
        self.enroll()
        ledger = json.loads((self.output / "versions.json").read_text())
        self.assertEqual(ledger["existing"], self.original["existing"])
        self.assertEqual((self.output / "ws-gfx" / "sample.tiles").read_bytes(), bytes(16))
        frozen = (self.output / "ws-gfx" / "spec.json").read_bytes()
        with self.assertRaises(ValueError):
            self.enroll()
        self.assertEqual((self.output / "ws-gfx" / "spec.json").read_bytes(), frozen)

    def test_source_digest_mismatch_leaves_no_spec(self):
        self.record["expect"]["sha256"] = "b" * 64
        with self.assertRaises(ValueError):
            self.enroll()
        self.assertFalse((self.output / "ws-gfx").exists())

    def test_traversal_and_reserved_sidecars_are_rejected(self):
        for name in ("../sample.tiles", "enrollment.json", ".hidden.tiles"):
            self.record["name"] = name
            with self.assertRaises(ValueError):
                self.enroll()
        self.assertFalse((self.output / "ws-gfx").exists())

    def test_duplicate_digest_does_not_replace_existing_provenance(self):
        digest = self.record["expect"]["sha256"]
        ledger = {digest: {"format": "ws-rom", "expectation": "frozen"}}
        (self.output / "versions.json").write_text(json.dumps(ledger))
        with self.assertRaises(ValueError):
            self.enroll()
        self.assertEqual(json.loads((self.output / "versions.json").read_text()), ledger)
        self.assertFalse((self.output / "ws-gfx").exists())
