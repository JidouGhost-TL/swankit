"""Synthetic enrollment checks: independent hashes, external root, no overwrite."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


STAGER = Path(__file__).resolve().parents[1] / "stage_sound.py"
spec = importlib.util.spec_from_file_location("sound_stager", STAGER)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Enrollment(unittest.TestCase):
    def inputs(self, root):
        source = root / "synthetic.raw"
        source.write_bytes(bytes(range(16)))
        reference = root / "oracle.txt"
        reference.write_text("independent synthetic expectation\n")
        manifest = root / "manifest.json"
        row = {"name": "synthetic.raw", "source": str(source),
               "args": {"references": [{"path": str(reference),
                                        "sha256": hashlib.sha256(reference.read_bytes()).hexdigest()}]},
               "expect": {"sha256": hashlib.sha256(source.read_bytes()).hexdigest()}}
        manifest.write_text(json.dumps({"samples": [row]}))
        return manifest, row

    def test_preserves_existing_specs_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory(prefix="sound-enrollment-") as temporary:
            root = Path(temporary)
            manifest, _ = self.inputs(root)
            output = root / "staging"
            output.mkdir()
            original = output / "versions.json"
            original.write_text("frozen root ledger\n")
            self.assertEqual(module.stage_sound(manifest, output), 1)
            frozen = (output / "ws-sound/spec.json").read_bytes()
            with self.assertRaisesRegex(ValueError, "already exists"):
                module.stage_sound(manifest, output)
            self.assertEqual(original.read_text(), "frozen root ledger\n")
            self.assertEqual((output / "ws-sound/spec.json").read_bytes(), frozen)

    def test_changed_reference_is_rejected_before_creation(self):
        with tempfile.TemporaryDirectory(prefix="sound-enrollment-") as temporary:
            root = Path(temporary)
            manifest, row = self.inputs(root)
            Path(row["args"]["references"][0]["path"]).write_text("changed\n")
            with self.assertRaisesRegex(ValueError, "reference hash changed"):
                module.stage_sound(manifest, root / "staging")
            self.assertFalse((root / "staging").exists())

    def test_traversal_and_repository_output_are_rejected(self):
        with tempfile.TemporaryDirectory(prefix="sound-enrollment-") as temporary:
            root = Path(temporary)
            manifest, row = self.inputs(root)
            with self.assertRaisesRegex(ValueError, "outside"):
                module.stage_sound(manifest, STAGER.parent.parent / "synthetic-staging")
            row["name"] = "../synthetic.raw"
            manifest.write_text(json.dumps({"samples": [row]}))
            with self.assertRaisesRegex(ValueError, "one path component"):
                module.stage_sound(manifest, root / "staging")


if __name__ == "__main__":
    unittest.main()
