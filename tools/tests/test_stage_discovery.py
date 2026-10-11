"""Independent witness enrollment must preserve frozen selections and identity."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('stage_discovery', TOOLS/'stage_discovery.py')
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

class DiscoveryStagingTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.output = self.root/'selection'
        self.rom = self.root/'image.fixture'
        self.rom.write_bytes(bytes(range(256)))
        self.raw = self.root/'capture.fixture'
        self.raw.write_bytes(bytes(range(64)))
        self.stored = self.root/'stream.fixture'
        self.stored.write_bytes(b'compressed-fixture')
        self.amap = self.root/'map.json'
        self.amap.write_text(json.dumps({'rom': {'sha256': self.hash(self.rom)}, 'assets': [{'id':'a'}]}))
        self.evidence = self.root/'evidence.json'
        self.evidence.write_text('{}')
        self.manifest = self.root/'manifest.json'
        self.doc = {'drafts':[{'rom':str(self.rom), 'map':str(self.amap), 'evidence':str(self.evidence),
                    'independent':{'a':{'path':str(self.raw),'sha256':self.hash(self.raw)}}}],
                    'codecs':[{'codec':'lzss-u16-zero','raw':str(self.raw),'stored':str(self.stored),
                    'raw_sha256':self.hash(self.raw),'stored_sha256':self.hash(self.stored),
                    'destination':0x2000,'prehistory':'low8-prefix','provenance':'synthetic capture'}]}
    def hash(self, path):
        return hashlib.sha256(path.read_bytes()).hexdigest()
    def enroll(self):
        self.manifest.write_text(json.dumps(self.doc))
        return mod.stage_discovery(self.manifest, self.output)
    def test_frozen_enrollment_cannot_be_overwritten(self):
        self.assertEqual(self.enroll(), (1,1))
        frozen = (self.output/'ws-discover/spec.json').read_bytes()
        self.assertTrue((self.output/'ws-discover/sample-000000.rom').is_symlink())
        codec = json.loads((self.output/'ws-compress-extra/spec.json').read_text())
        self.assertEqual(codec['samples']['sample-000000.pair']['args']['prehistory'], 'low8-prefix')
        with self.assertRaises(ValueError): self.enroll()
        self.assertEqual((self.output/'ws-discover/spec.json').read_bytes(), frozen)
    def test_changed_or_unknown_oracle_is_rejected_before_mutation(self):
        self.doc['drafts'][0]['independent']['a']['sha256'] = '0'*64
        with self.assertRaises(ValueError): self.enroll()
        self.assertFalse((self.output/'ws-discover').exists())
        self.doc['drafts'][0]['independent'] = {'unknown':{'path':str(self.raw),'sha256':self.hash(self.raw)}}
        with self.assertRaises(ValueError): self.enroll()
    def test_wrong_rom_and_invalid_policy_are_rejected(self):
        self.rom.write_bytes(b'changed')
        with self.assertRaises(ValueError): self.enroll()
        self.amap.write_text(json.dumps({'rom':{'sha256':self.hash(self.rom)},'assets':[{'id':'a'}]}))
        self.doc['codecs'][0]['destination'] = 65536
        with self.assertRaises(ValueError): self.enroll()
        self.doc['codecs'][0]['destination'] = 0x2000
        self.doc['codecs'][0]['prehistory'] = 'invented'
        with self.assertRaises(ValueError): self.enroll()
