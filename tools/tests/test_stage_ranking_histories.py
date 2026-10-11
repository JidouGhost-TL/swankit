"""History-backed ranking vectors preserve frozen specs and bind all inputs."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from stage_wondergate_server import stage_wondergate_server


class RankingEnrollment(unittest.TestCase):
    def test_history_digest_shape_and_replacement_are_checked(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            request = root / "request"
            reply = root / "reply"
            history = root / "history.json"
            manifest = root / "manifest.json"
            request.write_bytes(b"synthetic request")
            reply.write_bytes(b"synthetic reply")
            row = {"scope": {"definition": "synthetic", "board": "main", "course": "1"},
                   "identity": [65], "name": [66], "score": 7, "extra": 0,
                   "submitted_at": 0}
            history.write_text(json.dumps([row]))
            record = {"protocol": "HTTP", "clock": "20261010123456", "seed": 1,
                      "error": False, "oracle": "independent synthetic history",
                      "input_path": str(request), "response_path": str(reply),
                      "input_sha256": hashlib.sha256(request.read_bytes()).hexdigest(),
                      "response_sha256": hashlib.sha256(reply.read_bytes()).hexdigest(),
                      "response_length": len(reply.read_bytes()), "store_path": str(history),
                      "store_sha256": hashlib.sha256(history.read_bytes()).hexdigest()}

            def save(value):
                manifest.write_text(json.dumps({"schema": "swankit-wondergate-ranking-manifest/1",
                                                "samples": [value]}))

            destination = root / "staging"
            for key, value in (("store_sha256", "0" * 64), ("protocol", "BPL"),
                               ("store_path", "")):
                save(dict(record, **{key: value}))
                with self.assertRaises(ValueError):
                    stage_wondergate_server(manifest, destination)
                self.assertFalse(destination.exists())
            for invalid in (dict(row, score=-1), dict(row, score=True),
                            dict(row, identity=[256]), dict(row, submitted_at=None),
                            dict(row, scope={"definition": ""})):
                history.write_text(json.dumps([invalid]))
                save(dict(record, store_sha256=hashlib.sha256(history.read_bytes()).hexdigest()))
                with self.assertRaises(ValueError):
                    stage_wondergate_server(manifest, destination)
                self.assertFalse(destination.exists())
            history.write_text(json.dumps([row]))
            save(record)
            self.assertEqual(stage_wondergate_server(manifest, destination), 1)
            spec_path = destination / "ws-wondergate-rankings" / "spec.json"
            frozen = spec_path.read_bytes()
            sample = json.loads(frozen)["samples"]["sample-000000.request"]
            self.assertEqual(sample["args"]["store_sha256"], record["store_sha256"])
            self.assertEqual(sample["args"]["response_path"], str(reply))
            with self.assertRaisesRegex(ValueError, "never be overwritten"):
                stage_wondergate_server(manifest, destination)
            self.assertEqual(spec_path.read_bytes(), frozen)
