"""Original-source enrollment excludes external bytes and preserves frozen specs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('stage_terminated', TOOLS / 'stage_terminated.py')
STAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGE)


class TerminatedStagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.out = root / 'corpus'
        self.rom = root / 'source.dat'
        self.raw = root / 'decoded.dat'
        self.rom.write_bytes(b'XY\x00\x02\x00abZ')
        self.raw.write_bytes(b'ab')
        digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
        self.manifest = {'codec': 'lz-word16-msb', 'samples': [{
            'rom': {'path': str(self.rom), 'sha256': digest(self.rom)},
            'offset': 2, 'length': 5,
            'stored_sha256': hashlib.sha256(b'\x00\x02\x00ab').hexdigest(),
            'raw': {'path': str(self.raw), 'sha256': digest(self.raw)},
            'provenance': 'independent complete output'}]}
        self.path = root / 'manifest.json'

    def stage(self):
        self.path.write_text(json.dumps(self.manifest))
        return STAGE.stage_terminated(self.path, self.out)

    def test_exact_payload_excludes_prefix_tail_and_frozen_selection_refuses_reuse(self):
        self.assertEqual(self.stage(), 1)
        folder = self.out / 'ws-compress-word16-msb'
        pair = (folder / 'sample-000000.pair').read_bytes()
        label, provenance, raw, stored = struct.unpack('<IIQQ', pair[8:32])
        self.assertEqual((raw, stored), (2, 5))
        self.assertEqual(pair[32 + label + provenance:], b'ab\x00\x02\x00ab')
        before = (folder / 'spec.json').read_bytes()
        with self.assertRaises(ValueError):
            self.stage()
        self.assertEqual((folder / 'spec.json').read_bytes(), before)

    def test_wrong_original_or_oracle_digest_refuses_before_selection_write(self):
        self.raw.write_bytes(b'ac')
        with self.assertRaises(ValueError):
            self.stage()
        self.assertFalse((self.out / 'ws-compress-word16-msb').exists())

    def test_typed_source_bounds_and_stored_hash_are_required(self):
        for change in ({'offset': True}, {'length': 100}, {'stored_sha256': '0' * 64}):
            original = self.manifest['samples'][0].copy()
            self.manifest['samples'][0].update(change)
            with self.assertRaises(ValueError):
                self.stage()
            self.assertFalse((self.out / 'ws-compress-word16-msb').exists())
            self.manifest['samples'][0] = original


if __name__ == '__main__':
    unittest.main()
