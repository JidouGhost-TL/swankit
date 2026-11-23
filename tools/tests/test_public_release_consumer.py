"""The independent public consumer must refuse substituted or incomplete proof."""

import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "public-release-consumer.py"
SPEC = importlib.util.spec_from_file_location("public_release_consumer", SCRIPT)
CONSUMER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONSUMER)


class ConsumerTests(unittest.TestCase):
    target = "linux-x86_64"
    host = {"system": "Linux", "machine": "x86_64", "release": "fixture", "python": "3.13.0"}

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.work = Path(temp.name) / "consumer"
        self.binary = b"synthetic executable bytes"
        self.smoke = b"synthetic reviewed smoke bytes"
        archive, member, _, _, _ = CONSUMER.target_files(self.target)
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:gz") as bundle:
            info = tarfile.TarInfo(member)
            info.mode = 0o755
            info.size = len(self.binary)
            bundle.addfile(info, io.BytesIO(self.binary))
        self.archive = stream.getvalue()
        target = CONSUMER.TARGETS[self.target]
        self.target_patch = patch.dict(CONSUMER.TARGETS, {
            self.target: (*target[:4], CONSUMER.sha(self.archive), CONSUMER.sha(self.binary)),
        })
        self.target_patch.start()
        self.addCleanup(self.target_patch.stop)
        self.smoke_patch = patch.object(CONSUMER, "SMOKE_SHA", CONSUMER.sha(self.smoke))
        self.smoke_patch.start()
        self.addCleanup(self.smoke_patch.stop)
        self.host_patch = patch.object(CONSUMER, "host", return_value=self.host)
        self.host_patch.start()
        self.addCleanup(self.host_patch.stop)
        self.archive_name = archive

    def write_proof(self, **overrides):
        self.work.mkdir()
        (self.work / self.archive_name).write_bytes(self.archive)
        (self.work / "swankit").write_bytes(self.binary)
        (self.work / "release-smoke.py").write_bytes(self.smoke)
        proof = {**CONSUMER.identity(self.target), "checks": sorted(CONSUMER.CHECKS), "outputs": {}, **overrides}
        (self.work / "RECEIPT.json").write_text(json.dumps(proof), encoding="utf-8")

    def test_noop_download_cannot_replace_pinned_source(self):
        noop = b"def cli_smoke(*args): return []"
        with patch.object(CONSUMER, "obtain", side_effect=[self.archive, noop]):
            with self.assertRaisesRegex(ValueError, "released smoke SHA-256 mismatch"):
                CONSUMER.replay(self.target, self.work)
        self.assertFalse(self.work.exists())

    def test_changed_source_refuses_even_with_plausible_receipt(self):
        self.write_proof()
        (self.work / "release-smoke.py").write_text("pass\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "released smoke SHA-256 mismatch"):
            CONSUMER.verify_receipt(self.target, self.work)

    def test_local_bypass_does_not_qualify(self):
        self.write_proof(acquisition="local-bypass")
        with self.assertRaisesRegex(ValueError, "does not qualify"):
            CONSUMER.verify_receipt(self.target, self.work)

    def test_inspected_bytes_are_not_native_execution(self):
        self.write_proof(status="bytes-verified-only")
        with self.assertRaisesRegex(ValueError, "does not qualify"):
            CONSUMER.verify_receipt(self.target, self.work)

    def test_missing_runtime_checks_do_not_qualify(self):
        self.write_proof(checks=[])
        with self.assertRaisesRegex(ValueError, "missing, duplicated or unexpected"):
            CONSUMER.verify_receipt(self.target, self.work)

    def test_claimed_checks_without_output_do_not_qualify(self):
        self.write_proof()
        with self.assertRaises(FileNotFoundError):
            CONSUMER.verify_receipt(self.target, self.work)

    def test_valid_list_must_not_contain_a_duplicate(self):
        with self.assertRaisesRegex(ValueError, "missing, duplicated or unexpected"):
            CONSUMER.check_list(self.target, sorted(CONSUMER.CHECKS) + ["version"])


class WorkflowPins(unittest.TestCase):
    def test_windows_hardware_architecture_does_not_mask_process_architecture(self):
        with patch.object(CONSUMER.platform, "system", return_value="Windows"), \
                patch.object(CONSUMER.platform, "machine", return_value="AMD64"), \
                patch.object(CONSUMER.struct, "calcsize", return_value=8), \
                patch.dict(os.environ, {"PROCESSOR_ARCHITECTURE": "ARM64"}):
            with self.assertRaisesRegex(ValueError, "Windows process architecture differs"):
                CONSUMER.host("windows-x86_64")

    def test_32_bit_process_does_not_qualify(self):
        with patch.object(CONSUMER.platform, "system", return_value="Linux"), \
                patch.object(CONSUMER.platform, "machine", return_value="x86_64"), \
                patch.object(CONSUMER.struct, "calcsize", return_value=4):
            with self.assertRaisesRegex(ValueError, "pointer width differs"):
                CONSUMER.host("linux-x86_64")

    def test_workflow_pins_replay_and_verification_code(self):
        workflow = (ROOT / ".github/workflows/public-release-consumer.yml").read_text()
        self.assertIn(f"CONSUMER_SHA: '{CONSUMER.sha(SCRIPT.read_bytes())}'", workflow)
        self.assertIn("permissions: {}", workflow)
        for _, _, _, _, archive, executable in CONSUMER.TARGETS.values():
            self.assertIn(f"archive: '{archive}'", workflow)
            self.assertIn(f"executable: '{executable}'", workflow)


if __name__ == "__main__":
    unittest.main()
