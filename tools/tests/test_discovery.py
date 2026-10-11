import importlib.util
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('discovery', ROOT / 'discover-assets.py')
adapter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(adapter)


class ProfileTests(unittest.TestCase):
    def test_far_pointer_header_addend_and_terminator(self):
        rom = bytearray(128)
        rom[0:4] = bytes([16, 0, 0, 48])  # segment 3000 + offset 16
        rom[17:22] = b'HELLO'
        rom[22] = 0
        p = {'text_tables': [{'offset': 0, 'count': 1, 'stride': 4,
              'encoding': {'width': 4, 'form': 'far', 'add': -1},
              'window': 0, 'segment_base': 0x3000, 'terminator': '00',
              'routine': 'reader', 'charmap': {'builtin': 'explicit-profile'}}]}
        got = adapter.table_texts(bytes(rom), p)
        self.assertEqual((got[0]['offset'], got[0]['length']), (17, 6))
        self.assertEqual(got[0]['references'][0]['add'], -1)

    def test_pointer_profile_bounds(self):
        p = {'text_tables': [{'offset': 0, 'count': 1, 'stride': 2,
              'encoding': {'width': 2}, 'terminator': '00',
              'routine': 'reader', 'charmap': {}}]}
        with self.assertRaises(ValueError):
            adapter.table_texts(bytes([255, 255]), p)

    def test_font_candidate_needs_observed_source_and_run(self):
        rom = bytes(range(48))
        ram = bytearray(65536)
        for i, b in enumerate(rom):
            ram[0x2000+i*2] = b
        self.assertEqual(adapter.font_runs(rom, ram, bytes(256), []), [])
        got = adapter.font_runs(rom, ram, bytes(256),
                                [{'source': 0, 'length': 48, 'routine': 'expand'}])
        self.assertEqual(got[0]['format']['count'], 6)
        self.assertEqual(got[0]['format']['bpp'], 1)
