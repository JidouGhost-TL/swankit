"""Frozen adapter enrollment validates all evidence before changing selections."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from stage_wondergate_adapter import frames, stage_wondergate_adapter, tap_digest


def sha(data):
    return hashlib.sha256(data).hexdigest()


class AdapterEnrollment(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "staging"
        frozen = self.output / "ws-rom" / "spec.json"
        frozen.parent.mkdir(parents=True)
        frozen.write_text("frozen prior selection\n")
        self.tap = self.root / "serial.log"
        self.tap.write_bytes(b"T 0 # tap start\nT 0 > 02000101\nT 1 < 0200020200\n")
        self.expect = {
            "sha256": sha(self.tap.read_bytes()),
            "input_sha256": sha(bytes.fromhex("02000101")), "input_length": 4,
            "output_sha256": sha(bytes.fromhex("0200020200")), "output_length": 5,
            "input_frames": [{"offset": 0, "kind": 2, "cmd": 1, "params_length": 0, "params_sha256": sha(b"")}],
            "output_frames": [{"offset": 0, "kind": 2, "cmd": 2, "params_length": 1, "params_sha256": sha(b"\0")}],
            "network": {"expected": 0, "connected": 0, "closed": 0, "matched_requests": 0, "mismatches": 0},
        }
        self.record = {"id": "synthetic-status", "oracle": "independent status wire contract",
                       "tap_path": str(self.tap), "tap_sha256": self.expect["sha256"],
                       "profile": "revival", "header": "len16be", "resolutions": [], "peers": [],
                       "expect": self.expect}
        self.manifest = self.root / "manifest.json"

    def save(self, records=None, exclusions=None):
        self.manifest.write_text(json.dumps({"schema": "swankit-wondergate-adapter-manifest/1",
            "samples": records if records is not None else [self.record],
            "source_excluded": exclusions or []}))

    def snapshot(self):
        return {str(p.relative_to(self.output)): p.read_bytes() for p in self.output.rglob("*") if p.is_file()}

    def test_enrollment_preserves_every_prior_file_and_refuses_overwrite(self):
        self.save()
        before = self.snapshot()
        self.assertEqual(stage_wondergate_adapter(self.manifest, self.output), 1)
        for path, data in before.items():
            self.assertEqual(self.snapshot()[path], data)
        directory = self.output / "ws-wondergate-adapter"
        sample = json.loads((directory / "spec.json").read_text())["samples"]["sample-000000.tap"]
        self.assertEqual(sample["expect"], self.expect)
        self.assertEqual((directory / "sample-000000.tap").resolve(), self.tap)
        frozen = self.snapshot()
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            stage_wondergate_adapter(self.manifest, self.output)
        self.assertEqual(self.snapshot(), frozen)

    def test_invalid_last_record_cannot_partially_enroll(self):
        before = self.snapshot()
        for key, value in (("tap_sha256", "0" * 64), ("oracle", ""), ("profile", "unknown"),
                           ("header", "unknown"), ("resolutions", [None]), ("peers", [None]),
                           ("expect", {}), ("id", "")):
            with self.subTest(key=key):
                invalid = dict(self.record, id="invalid", **({key: value} if key != "id" else {}))
                if key == "id":
                    invalid["id"] = value
                self.save([self.record, invalid])
                with self.assertRaises(ValueError):
                    stage_wondergate_adapter(self.manifest, self.output)
                self.assertEqual(self.snapshot(), before)

    def test_old_and_failed_captures_need_hash_and_reason(self):
        old = self.root / "old.log"
        old.write_bytes(b"incomplete historical tap")
        excluded = {"path": str(old), "sha256": sha(old.read_bytes()), "reason": "incomplete capture"}
        before = self.snapshot()
        for bad in (dict(excluded, reason=""), dict(excluded, sha256="0" * 64),
                    dict(excluded, path=str(self.tap), sha256=self.expect["sha256"])):
            self.save(exclusions=[bad])
            with self.assertRaises(ValueError):
                stage_wondergate_adapter(self.manifest, self.output)
            self.assertEqual(self.snapshot(), before)
        self.save(exclusions=[excluded])
        stage_wondergate_adapter(self.manifest, self.output)
        spec = json.loads((self.output / "ws-wondergate-adapter/spec.json").read_text())
        self.assertEqual(spec["source_excluded"], {str(old): "incomplete capture"})
        self.assertFalse((self.output / "ws-wondergate-adapter/old.log").exists())

    def test_peer_vectors_are_checked_and_frozen_without_runtime_paths(self):
        request, response = self.root / "request.bin", self.root / "response.bin"
        request.write_bytes(b"request\0")
        response.write_bytes(b"response\xff")
        peer = {"ip": [127, 0, 0, 1], "port": 80, "request_path": str(request), "response_path": str(response),
                "request_sha256": sha(request.read_bytes()), "response_sha256": sha(response.read_bytes())}
        record = copy.deepcopy(self.record)
        record["peers"] = [peer]
        record["expect"]["network"] = {"expected": 1, "connected": 1, "closed": 1, "matched_requests": 1, "mismatches": 0}
        before = self.snapshot()
        for change in ({"request_sha256": "0" * 64}, {"response_sha256": "0" * 64},
                       {"port": True}, {"port": 65536}, {"ip": [0, 0, 0, 256]}):
            invalid = copy.deepcopy(record)
            invalid["peers"][0].update(change)
            self.save([invalid])
            with self.assertRaises(ValueError):
                stage_wondergate_adapter(self.manifest, self.output)
            self.assertEqual(self.snapshot(), before)
        self.save([record])
        stage_wondergate_adapter(self.manifest, self.output)
        spec = json.loads((self.output / "ws-wondergate-adapter/spec.json").read_text())
        self.assertEqual(spec["samples"]["sample-000000.tap"]["args"]["peers"],
            [{"ip": [127, 0, 0, 1], "port": 80, "request_hex": request.read_bytes().hex(), "response_hex": response.read_bytes().hex()}])

    def test_independent_grammar_checks_lengths_padding_and_timestamps(self):
        params = bytes(299)
        wire = bytes.fromhex("11012c0e") + params
        self.assertEqual(frames(b"\0" + wire, "len16be", True)[0]["offset"], 1)
        self.assertEqual(frames(wire, "len16be", True)[0]["params_length"], 299)
        for bad in (wire[:-1], b"\x11\0\0\x01", b"\x11\x10\x01\x01"):
            with self.assertRaises(ValueError):
                frames(bad, "len16be", True)
        with self.assertRaises(ValueError):
            frames(wire, "zero-then-len8", False)
        for tap in (b"T 18446744073709551616 > 02000101\n", b"T 0 > xy\n", b"T 0 ! 00\n", b"\xff"):
            with self.assertRaises(ValueError):
                tap_digest(tap, "len16be", True)

    def test_shapes_bounds_duplicates_and_repo_destination_are_rejected(self):
        before = self.snapshot()
        for value in ([], {}, {"schema": "unknown"}, {"schema": "swankit-wondergate-adapter-manifest/1", "samples": [None]}):
            self.manifest.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                stage_wondergate_adapter(self.manifest, self.output)
            self.assertEqual(self.snapshot(), before)
        self.save([self.record, self.record])
        with self.assertRaisesRegex(ValueError, "unique"):
            stage_wondergate_adapter(self.manifest, self.output)
        with self.manifest.open("wb") as file:
            file.truncate(8 * 1024 * 1024 + 1)
        with self.assertRaisesRegex(ValueError, "oversized"):
            stage_wondergate_adapter(self.manifest, self.output)
        self.save()
        with self.assertRaisesRegex(ValueError, "outside the repository"):
            stage_wondergate_adapter(self.manifest, Path(__file__).resolve().parents[2] / "staging")

    def test_public_entrypoint_enrolls_the_adapter_namespace(self):
        self.save()
        tool = Path(__file__).resolve().parents[1] / "stage-corpus.py"
        result = subprocess.run([sys.executable, str(tool), "--wondergate-manifest", str(self.manifest),
                                 "--output", str(self.output)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.output / "ws-wondergate-adapter/spec.json").is_file())

    def test_boolean_and_float_counts_cannot_create_a_frozen_selection(self):
        before = self.snapshot()
        for value in (False, 0.0):
            invalid = copy.deepcopy(self.record)
            invalid["expect"]["network"]["mismatches"] = value
            self.save([invalid])
            with self.assertRaisesRegex(ValueError, "frozen adapter expectation"):
                stage_wondergate_adapter(self.manifest, self.output)
            self.assertEqual(self.snapshot(), before)

    def test_broken_namespace_symlink_is_never_replaced(self):
        self.save()
        namespace = self.output / "ws-wondergate-adapter"
        namespace.symlink_to(self.root / "missing", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "never be overwritten"):
            stage_wondergate_adapter(self.manifest, self.output)
        self.assertTrue(namespace.is_symlink())
        self.assertEqual(namespace.readlink(), self.root / "missing")

    def test_peer_retention_budget_covers_the_entire_manifest(self):
        request, response = self.root / "request.bin", self.root / "response.bin"
        request.write_bytes(b"abc")
        response.write_bytes(b"def")
        peer = {"ip": [127, 0, 0, 1], "port": 80, "request_path": str(request), "response_path": str(response),
                "request_sha256": sha(b"abc"), "response_sha256": sha(b"def")}
        record = copy.deepcopy(self.record)
        record["peers"] = [peer]
        record["expect"]["network"] = {"expected": 1, "connected": 1, "closed": 1, "matched_requests": 1, "mismatches": 0}
        self.save([record, dict(record, id="another-capture")])
        before = self.snapshot()
        with patch("stage_wondergate_adapter.MAX_RETAINED_PEERS", 10):
            with self.assertRaisesRegex(ValueError, "retention exceeds bound"):
                stage_wondergate_adapter(self.manifest, self.output)
        self.assertEqual(self.snapshot(), before)
