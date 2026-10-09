"""Product enrollment preserves owner expectations and rejects corrupted bytes."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

MODULE_PATH = Path(__file__).resolve().parents[1] / "stage_product.py"
SPEC = importlib.util.spec_from_file_location("stage_product", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ProductStaging(unittest.TestCase):
    def test_enrollment_is_independent_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = {}
            for selection, data in (("ws-rom", b"synthetic-rom"), ("ws-patch", b"PATCHEOF")):
                folder = root / selection
                folder.mkdir()
                (folder / "input").write_bytes(data)
                spec = {"format": selection, "samples": {"input": {
                    "expect": {"sha256": hashlib.sha256(data).hexdigest()}}}}
                original[selection] = json.dumps(spec).encode()
                (folder / "spec.json").write_bytes(original[selection])
            self.assertEqual(MODULE.stage_product(root), 2)
            output = json.loads((root / "swankit-detection" / "spec.json").read_bytes())
            self.assertEqual({s["expect"]["selected"] for s in output["samples"].values()},
                             {"ws-rom", "ws-patch-ips"})
            for selection, data in original.items():
                self.assertEqual((root / selection / "spec.json").read_bytes(), data)
            frozen = (root / "swankit-detection" / "spec.json").read_bytes()
            with self.assertRaisesRegex(ValueError, "cannot be overwritten"):
                MODULE.stage_product(root)
            self.assertEqual((root / "swankit-detection" / "spec.json").read_bytes(), frozen)

    def test_hash_mismatch_leaves_no_partial_selection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            folder = root / "ws-rom"
            folder.mkdir()
            (folder / "input").write_bytes(b"changed")
            (folder / "spec.json").write_text(json.dumps({"format": "ws-rom", "samples": {
                "input": {"expect": {"sha256": "0" * 64}}}}))
            with self.assertRaisesRegex(ValueError, "frozen hash"):
                MODULE.stage_product(root)
            self.assertFalse((root / "swankit-detection").exists())
