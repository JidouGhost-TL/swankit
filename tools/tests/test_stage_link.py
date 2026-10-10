"""Admission checks preserve frozen link and unrelated corpus selections."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from stage_link import stage_link


class LinkStaging(unittest.TestCase):
    def fixture(self, root):
        source = root / "wire.dat"
        reference = root / "reference.dat"
        source.write_bytes(bytes(range(256)))
        reference.write_bytes(source.read_bytes())
        manifest = root / "oracle.json"
        record = {
            "input_path": str(source), "reference_path": str(reference),
            "input_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
            "output_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
            "output_bytes": 256, "direction": "B>A", "latency_ms": 20,
            "jitter_ms": 10, "seed": 22, "oracle": "independent binary sequence",
        }
        manifest.write_text(json.dumps([record]))
        return manifest, record

    def test_enrollment_preserves_frozen_specs_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, _ = self.fixture(root)
            other = root / "ws-rom/spec.json"
            other.parent.mkdir()
            other.write_bytes(b"frozen unrelated selection")
            stage_link(manifest, root)
            frozen = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
            spec = json.loads((root / "ws-link/spec.json").read_text())
            self.assertEqual(spec["samples"]["sample-000000.dat"]["expect"]["forwarded"], [0, 256])
            with self.assertRaisesRegex(ValueError, "frozen"):
                stage_link(manifest, root)
            self.assertEqual(frozen, {p: p.read_bytes() for p in root.rglob("*") if p.is_file()})

    def test_rejects_mutated_wire_or_reference_before_writing(self):
        for field in ("input_path", "reference_path"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                manifest, record = self.fixture(root)
                Path(record[field]).write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "differ"):
                    stage_link(manifest, root)
                self.assertFalse((root / "ws-link").exists())

    def test_invalid_contract_creates_no_selection(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            manifest, record = self.fixture(root)
            for field, value in (("direction", "invalid"), ("latency_ms", True),
                                 ("latency_ms", 600001), ("jitter_ms", -1),
                                 ("seed", 1 << 64), ("output_bytes", True), ("oracle", "")):
                changed = dict(record)
                changed[field] = value
                manifest.write_text(json.dumps([changed]))
                with self.assertRaises(ValueError):
                    stage_link(manifest, root)
                self.assertFalse((root / "ws-link").exists())
