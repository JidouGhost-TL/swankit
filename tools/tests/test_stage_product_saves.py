"""Save product enrollment copies independent expectations and preserves owners."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

MODULE_PATH = Path(__file__).resolve().parents[1] / "stage_product_saves.py"
SPEC = importlib.util.spec_from_file_location("stage_product_saves", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SaveProductStaging(unittest.TestCase):
    def setup_root(self, root):
        repository = root / "repository"
        definitions = repository / "crates/ws-saves/data"
        definitions.mkdir(parents=True)
        (definitions / "synthetic.json").write_text(json.dumps({
            "schema": "swankit-save-def/1", "id": "synthetic", "validity": {"policy": "program", "exclude": ["working_copy"]}}))
        output = root / "corpus"
        output.mkdir()
        source = root / "natural.eeprom"
        source.write_bytes(bytes(range(128)))
        companion = root / "natural.ieep"
        companion.write_bytes(bytes([1, 2]))
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        for namespace in ("ws-saves", "ws-saves-parity"):
            folder = output / namespace
            folder.mkdir()
            (folder / "sample").symlink_to(source)
            sample = {"args": {"id": "synthetic", "name": "save.eeprom", "context": {}},
                      "expect": {"sha256": digest, "decoded": {"valid": True}}}
            if namespace.endswith("parity"):
                sample["args"]["corrupt_offset"] = 0
                sample["expect"].update(repaired={"valid": True}, repaired_sha256="f" * 64)
            (folder / "spec.json").write_text(json.dumps({
                "format": "ws-saves", "samples": {"sample": sample}}))
            (output / (namespace + "-enrollment.json")).write_text(json.dumps({
                "records": [{"id": "synthetic", "sha256": digest,
                             "name": "save.eeprom", "path": str(source)}]}))
        return repository, output, source

    def test_copy_natural_and_repair_expectations_and_refuse_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            repository, output, _ = self.setup_root(Path(directory))
            before = {p.relative_to(output): p.read_bytes() for p in output.rglob("*.json")}
            with mock.patch.object(MODULE, "__file__", str(repository / "tools/stage_product_saves.py")):
                self.assertEqual(MODULE.stage_product_saves(output), 2)
                spec = json.loads((output / "swankit-save-detection/spec.json").read_bytes())
                self.assertTrue(all(s["expect"]["matched"] for s in spec["samples"].values()))
                self.assertEqual(sum("repaired" in s["expect"] for s in spec["samples"].values()), 1)
                self.assertTrue(all(s["args"]["context"]["images"] for s in spec["samples"].values()))
                for path, original in before.items():
                    self.assertEqual((output / path).read_bytes(), original)
                frozen = (output / "swankit-save-detection/spec.json").read_bytes()
                with self.assertRaisesRegex(ValueError, "cannot be overwritten"):
                    MODULE.stage_product_saves(output)
                self.assertEqual((output / "swankit-save-detection/spec.json").read_bytes(), frozen)

    def test_changed_source_cannot_create_selection(self):
        with tempfile.TemporaryDirectory() as directory:
            repository, output, source = self.setup_root(Path(directory))
            source.write_bytes(b"changed")
            with mock.patch.object(MODULE, "__file__", str(repository / "tools/stage_product_saves.py")):
                with self.assertRaisesRegex(ValueError, "frozen source hash"):
                    MODULE.stage_product_saves(output)
            self.assertFalse((output / "swankit-save-detection").exists())

    def test_working_regions_and_global_rejection_do_not_supply_positive_evidence(self):
        self.assertEqual(MODULE._positive({"working_copy": {"valid": True}}, ["working_copy"]), 0)
        self.assertEqual(MODULE._positive({"slots": [{"valid": False}, {"valid": True}]}, []), 1)

    def test_invalid_validity_policy_cannot_create_selection(self):
        with tempfile.TemporaryDirectory() as directory:
            repository, output, _ = self.setup_root(Path(directory))
            definition = repository / "crates/ws-saves/data/synthetic.json"
            value = json.loads(definition.read_bytes())
            value["validity"]["policy"] = "unknown"
            definition.write_text(json.dumps(value))
            with mock.patch.object(MODULE, "__file__", str(repository / "tools/stage_product_saves.py")):
                with self.assertRaisesRegex(ValueError, "validity policy"):
                    MODULE.stage_product_saves(output)
            self.assertFalse((output / "swankit-save-detection").exists())
