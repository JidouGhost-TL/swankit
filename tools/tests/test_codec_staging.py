"""Synthetic enrollment checks for the frozen external codec namespace."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "stage-corpus.py"
SPEC = importlib.util.spec_from_file_location("stage_corpus", SCRIPT)
STAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGE)


class CodecStagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "external"
        self.record = {
            "codec": "lz-rle-7f", "consumed": 4, "require_fit": True,
            "provenance": "synthetic fixture", "evidence": "synthetic",
        }
        for kind, data in (("raw", b"A"), ("stored", b"\x80A\x7f\xff"),
                           ("encoded", b"\x80A\x7f\xff")):
            file = self.root / kind
            file.write_bytes(data)
            self.record[kind + "_path"] = str(file)
            self.record[kind + "_sha256"] = hashlib.sha256(data).hexdigest()
        self.manifest = self.root / "manifest.json"
        self.args = argparse.Namespace(output=self.output, codec_manifest=self.manifest, progress_log=None)

    def stage(self):
        self.manifest.write_text(json.dumps({"records": [self.record], "oracle_sources": {}}))
        STAGE.stage_codecs(self.args)

    def test_add_namespace_preserves_existing_specs_and_refuses_overwrite(self):
        existing = self.output / "unrelated" / "spec.json"
        existing.parent.mkdir(parents=True)
        existing.write_text("frozen unrelated spec")
        self.stage()
        frozen = (self.output / "ws-compress" / "spec.json").read_bytes()
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            self.stage()
        self.assertEqual((self.output / "ws-compress" / "spec.json").read_bytes(), frozen)
        self.assertEqual(existing.read_text(), "frozen unrelated spec")

    def test_mismatched_independent_hash_rejected_before_writing(self):
        self.record["raw_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "digest differs"):
            self.stage()
        self.assertFalse(self.output.exists())

    def test_required_slot_fit_rejected_before_writing(self):
        data = b"larger stream"
        Path(self.record["encoded_path"]).write_bytes(data)
        self.record["encoded_sha256"] = hashlib.sha256(data).hexdigest()
        with self.assertRaisesRegex(ValueError, "required original slot"):
            self.stage()
        self.assertFalse(self.output.exists())

    def test_existing_pair_set_rejected_without_changing_it(self):
        pairs = self.output / "_compress_pairs" / "lz-rle-7f"
        pairs.mkdir(parents=True)
        sentinel = pairs / "frozen.pair"
        sentinel.write_text("frozen")
        with self.assertRaisesRegex(ValueError, "pairs already exist"):
            self.stage()
        self.assertEqual(sentinel.read_text(), "frozen")
        self.assertFalse((self.output / "ws-compress").exists())

    def test_repository_destination_rejected(self):
        self.args.output = SCRIPT.parents[1] / "rejected-test-staging"
        with self.assertRaisesRegex(ValueError, "outside the repository"):
            self.stage()
        self.assertFalse(self.args.output.exists())
