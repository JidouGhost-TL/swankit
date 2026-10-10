"""Service enrollment validates independent evidence before writing specs."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from stage_wondergate_server import stage_wondergate_server


class ServicesEnrollment(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.root = root / "staging"
        frozen = self.root / "ws-rom" / "spec.json"
        frozen.parent.mkdir(parents=True)
        frozen.write_text("existing frozen expectation\n")
        self.request = root / "request"
        self.request.write_bytes(bytes([1, 24, 0, 0, 24, 0, 0, 0]) + bytes(16))
        self.response = root / "response"
        self.response.write_bytes(bytes([1, 24, 16, 0, 24, 0, 0, 0]) + bytes(16))
        self.record = {
            "protocol": "BPL", "clock": "20261005200403", "seed": 0, "error": False,
            "input_path": str(self.request), "response_path": str(self.response),
            "input_sha256": hashlib.sha256(self.request.read_bytes()).hexdigest(),
            "response_sha256": hashlib.sha256(self.response.read_bytes()).hexdigest(),
            "response_length": 24, "oracle": "independent synthetic PHASE0 contract",
        }
        self.manifest = root / "manifest.json"

    def save(self, record=None):
        self.manifest.write_text(json.dumps({"schema": "swankit-wondergate-services-manifest/1",
                                             "samples": [record or self.record]}))

    def frozen(self):
        return {p.relative_to(self.root): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}

    def test_enroll_preserves_prior_specs_and_refuses_replacement(self):
        self.save()
        before = self.frozen()
        self.assertEqual(stage_wondergate_server(self.manifest, self.root), 1)
        after = self.frozen()
        for path, data in before.items():
            self.assertEqual(after[path], data)
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            stage_wondergate_server(self.manifest, self.root)
        self.assertEqual(self.frozen(), after)

    def test_invalid_evidence_is_rejected_before_staging(self):
        before = self.frozen()
        for key, value in (("input_sha256", "0" * 64), ("response_sha256", "0" * 64),
                           ("response_length", 8), ("protocol", "unknown"),
                           ("clock", "20260229000000"), ("seed", -1),
                           ("seed", True), ("error", 0), ("oracle", "")):
            with self.subTest(key=key, value=value):
                self.save(dict(self.record, **{key: value}))
                with self.assertRaises(ValueError):
                    stage_wondergate_server(self.manifest, self.root)
                self.assertEqual(self.frozen(), before)

    def title_manifest(self, records=None):
        record = dict(self.record, protocol="HTTP", store=[])
        self.manifest.write_text(json.dumps({"schema": "swankit-wondergate-server-manifest/1",
                                             "samples": records if records is not None else [record]}))

    def test_title_flows_get_separate_frozen_namespace(self):
        self.save()
        stage_wondergate_server(self.manifest, self.root)
        before = self.frozen()
        self.title_manifest()
        self.assertEqual(stage_wondergate_server(self.manifest, self.root), 1)
        for path, data in before.items():
            self.assertEqual(self.frozen()[path], data)
        directory = self.root / "ws-wondergate-server"
        spec = json.loads((directory / "spec.json").read_text())
        self.assertEqual(spec["format"], "ws-wondergate-server")
        sample = spec["samples"]["sample-000000.request"]
        self.assertEqual(sample["args"]["store"], [])
        self.assertEqual(sample["expect"]["response_sha256"], self.record["response_sha256"])
        self.assertEqual((directory / "sample-000000.request").resolve(), self.request)
        frozen = self.frozen()
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            stage_wondergate_server(self.manifest, self.root)
        self.assertEqual(self.frozen(), frozen)

    def test_title_flows_reject_unproven_state_and_partial_enrollment(self):
        before = self.frozen()
        valid = dict(self.record, protocol="HTTP", store=[])
        for invalid in (dict(valid, store=[{}]), dict(valid, protocol="BPL"),
                        dict(valid, response_sha256="0" * 64),
                        {k: v for k, v in valid.items() if k != "store"}):
            self.title_manifest([valid, invalid])
            with self.assertRaises(ValueError):
                stage_wondergate_server(self.manifest, self.root)
            self.assertEqual(self.frozen(), before)

    def test_invalid_manifest_shapes_and_oversized_files_do_not_stage(self):
        before = self.frozen()
        for manifest in ([], {}, {"schema": "unknown", "samples": []},
                         {"schema": "swankit-wondergate-server-manifest/1", "samples": [None]},
                         {"schema": "swankit-wondergate-server-manifest/1", "samples": [{}]}):
            self.manifest.write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):
                stage_wondergate_server(self.manifest, self.root)
            self.assertEqual(self.frozen(), before)
        with self.manifest.open("wb") as file:
            file.truncate(8 * 1024 * 1024 + 1)
        with self.assertRaisesRegex(ValueError, "oversized"):
            stage_wondergate_server(self.manifest, self.root)
        self.assertEqual(self.frozen(), before)

    def test_title_flow_destination_must_be_external(self):
        self.title_manifest()
        repository = Path(__file__).resolve().parents[2]
        with self.assertRaisesRegex(ValueError, "outside the repository"):
            stage_wondergate_server(self.manifest, repository / "synthetic-staging")
