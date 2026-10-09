"""Python enrollment preserves frozen values and rejects changed sources."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "stage_python.py"
spec = importlib.util.spec_from_file_location("stage_python", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PythonEnrollmentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.frozen = {}
        for name in ("swankit-detection", "swankit-save-detection"):
            folder = self.root / name
            folder.mkdir()
            source = folder / "source.dat"
            source.write_bytes(b"synthetic")
            value = {"format": name, "samples": {source.name: {
                "args": {"independent": name},
                "expect": {"sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "oracle": [1, 2]},
            }}}
            path = folder / "spec.json"
            path.write_text(json.dumps(value))
            self.frozen[path] = path.read_bytes()
        stored = self.root / "stored.dat"
        stored.write_bytes(b"stored")
        digest = hashlib.sha256(stored.read_bytes()).hexdigest()
        folder = self.root / "ws-compress"
        folder.mkdir()
        (folder / "spec.json").write_text(json.dumps({"format": "ws-compress", "samples": {
            "sample.pair": {"args": {"provenance": "synthetic"}, "expect": {
                "codec": "lz-rle-7f", "stored_sha256": digest, "raw_len": 1, "oracle": "unchanged",
            }}}}))
        (self.root / "ws-compress-enrollment.json").write_text(json.dumps({"records": [{
            "codec": "lz-rle-7f", "stored_sha256": digest, "provenance": "synthetic", "stored_path": str(stored),
        }]}))

    def test_copy_and_no_overwrite(self):
        self.assertEqual(module.stage_python(self.root), 3)
        directory = self.root / "swankit-python"
        first = (directory / "spec.json").read_bytes()
        enrolled = json.loads(first)
        self.assertEqual(enrolled["samples"]["sample-000002.dat"]["expect"]["oracle"], "unchanged")
        self.assertTrue((directory / "sample-000002.dat").is_symlink())
        with self.assertRaisesRegex(ValueError, "cannot be overwritten"):
            module.stage_python(self.root)
        self.assertEqual((directory / "spec.json").read_bytes(), first)
        for path, content in self.frozen.items():
            self.assertEqual(path.read_bytes(), content)

    def test_reject_changed_source_before_creating_namespace(self):
        (self.root / "swankit-detection/source.dat").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "frozen hash"):
            module.stage_python(self.root)
        self.assertFalse((self.root / "swankit-python").exists())

    def test_reject_changed_codec_source(self):
        (self.root / "stored.dat").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "frozen hash"):
            module.stage_python(self.root)
        self.assertFalse((self.root / "swankit-python").exists())

    def test_reject_nonlocal_owner_name(self):
        path = self.root / "swankit-detection/spec.json"
        value = json.loads(path.read_bytes())
        value["samples"]["../escape.dat"] = value["samples"].pop("source.dat")
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "single filename"):
            module.stage_python(self.root)
        self.assertFalse((self.root / "swankit-python").exists())
