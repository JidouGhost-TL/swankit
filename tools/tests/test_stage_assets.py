"""Asset-map enrollment preserves frozen selections and requires independent evidence."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


TOOLS = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("stage_assets", TOOLS / "stage_assets.py")
STAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGE)


class AssetStagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.output = root / "staging"
        (self.output / "existing").mkdir(parents=True)
        (self.output / "existing" / "spec.json").write_text('{"samples":{}}')
        self.rom = root / "image.fixture"
        self.rom.write_bytes(bytes(range(256)) * 256)
        self.evidence = root / "decoded.bin"
        self.evidence.write_bytes(bytes(range(16)))
        self.map = root / "map.json"
        self.map.write_text(json.dumps({
            "schema": "swankit-asset-map/1",
            "rom": {"sha256": self.digest(self.rom), "size": 65536},
            "assets": [{"id": "a", "kind": "other", "offset": 0, "length": 16}]}))
        self.row = {"name": "sample.rom", "rom": str(self.rom), "map": str(self.map),
                    "independent": {"a": {"path": str(self.evidence), "sha256": self.digest(self.evidence)}},
                    "provenance": {"map_source": "synthetic generator", "independent_tool": "synthetic"}}
        self.manifest = root / "manifest.json"

    def digest(self, path):
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def enroll(self, rows=None):
        self.manifest.write_text(json.dumps({"samples": rows if rows is not None else [self.row]}))
        return STAGE.stage_assets(self.manifest, self.output)

    def test_enrollment_writes_spec_and_refuses_overwrite(self):
        self.assertEqual(self.enroll(), 1)
        spec = json.loads((self.output / "ws-assets" / "spec.json").read_text())
        sample = spec["samples"]["sample.rom"]
        self.assertEqual(sample["expect"]["independent"], {"a": self.digest(self.evidence)})
        self.assertEqual(sample["args"]["map"], "maps/sample.json")
        self.assertTrue((self.output / "ws-assets" / "sample.rom").is_symlink())
        self.assertEqual((self.output / "existing" / "spec.json").read_text(), '{"samples":{}}')
        with self.assertRaises(ValueError):
            self.enroll()

    def test_changed_evidence_wrong_rom_and_unknown_assets_are_refused(self):
        bad = dict(self.row, independent={"a": {"path": str(self.evidence), "sha256": "0" * 64}})
        with self.assertRaises(ValueError):
            self.enroll([bad])
        bad = dict(self.row, independent={"zz": self.row["independent"]["a"]})
        with self.assertRaises(ValueError):
            self.enroll([bad])
        other = Path(self.temp.name) / "other.fixture"
        other.write_bytes(b"\0" * 65536)
        with self.assertRaises(ValueError):
            self.enroll([dict(self.row, rom=str(other))])
        self.assertFalse((self.output / "ws-assets").exists())

    def test_later_map_revisions_enroll_beside_the_frozen_selection(self):
        self.assertEqual(self.enroll(), 1)
        frozen = (self.output / "ws-assets" / "spec.json").read_text()
        self.manifest.write_text(json.dumps({"samples": [self.row]}))
        self.assertEqual(STAGE.stage_assets(self.manifest, self.output, "ws-assets-m27"), 1)
        self.assertTrue((self.output / "ws-assets-m27" / "maps" / "sample.json").is_file())
        self.assertEqual((self.output / "ws-assets" / "spec.json").read_text(), frozen)
        with self.assertRaises(ValueError):
            STAGE.stage_assets(self.manifest, self.output, "ws-assets-m27")
        with self.assertRaises(ValueError):
            STAGE.stage_assets(self.manifest, self.output, "../elsewhere")

    def test_staging_inside_the_repository_is_refused(self):
        self.manifest.write_text(json.dumps({"samples": [self.row]}))
        with self.assertRaises(ValueError):
            STAGE.stage_assets(self.manifest, TOOLS.parent / "corpus-in-repo")


if __name__ == "__main__":
    unittest.main()
