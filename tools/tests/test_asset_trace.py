import json
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from asset_trace import observations, reference_candidates, IMMEDIATE


class TraceTests(unittest.TestCase):
    def row(self, **changes):
        row = dict(kind='instruction-flow', id=7, entry=0xf0100, source=100,
                   source_end=200, destination=0x2000, length=32, explained_bytes=16,
                   causal_source_bytes=8, complete_return=True, output_hex=bytes(range(32)).hex(),
                   pointer_origins=[IMMEDIATE+11, IMMEDIATE+12, 400])
        row.update(changes)
        return json.dumps(row).encode()+b'\n'

    def test_instruction_witness_keeps_exact_extent_and_capture_identity(self):
        capture = self.row()
        got = observations(capture+capture)
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0]['output'], list(range(32)))
        self.assertEqual(got[0]['destination'], 0x2000)
        self.assertIn('entry=f0100', got[0]['routine'])
        self.assertEqual(got[0]['references'], [])

    def test_no_temporal_or_incomplete_witness_is_promoted(self):
        self.assertEqual(observations(self.row(kind='temporal')), [])
        self.assertEqual(observations(self.row(complete_return=False)), [])
        self.assertEqual(observations(self.row(explained_bytes=0)), [])
        self.assertEqual(observations(self.row(causal_source_bytes=0)), [])

    def test_bounds_and_whole_output_required(self):
        for changes in [dict(length=33), dict(destination=65530), dict(source_end=99)]:
            with self.assertRaises(ValueError):
                observations(self.row(**changes))

    def test_pointer_origins_are_candidates_without_reference_encoding(self):
        self.assertEqual(reference_candidates(self.row()), [
            dict(at=11, proof_kind='code-immediate'), dict(at=12, proof_kind='code-immediate'),
            dict(at=400, proof_kind='rom-read')])

    def test_offset_register_is_kept_separate_from_linear_ram_address(self):
        row = self.row(destination=0x2013, entry_registers={'DI':0x13,'ES':0x200})
        self.assertEqual(observations(row)[0]['codec_destination'],0x13)
        wrong = self.row(destination=0x2013, entry_registers={'DI':0x14,'ES':0x200})
        self.assertIsNone(observations(wrong)[0]['codec_destination'])

    def test_glyph_sized_call_remains_a_witness(self):
        row = self.row(length=8, explained_bytes=8, output_hex=bytes(range(8)).hex())
        self.assertEqual(len(observations(row)[0]['output']),8)

    def test_reference_field_candidates_require_all_origins_and_actual_target(self):
        from asset_trace import reference_fields
        rom=bytearray(0x80000);rom[11:13]=(100).to_bytes(2,'little')
        rows=reference_fields(self.row(),bytes(rom))
        self.assertEqual([(r['at'],r['width']) for r in rows],[(11,2)])
        rom[11]=101
        self.assertEqual(reference_fields(self.row(),bytes(rom)),[])
        rom[20:24]=bytes([0x40,0,0,0x20])
        row=self.row(source=0x10040,source_end=0x10080,pointer_origins=[20,21,22,23],banks=['0:1:0'])
        fields=reference_fields(row,bytes(rom))
        self.assertIn(dict(at=20,width=4,form='far',source=0x10040,proof_kind='rom-read'),fields)
        partial=self.row(source=0x10040,source_end=0x10080,pointer_origins=[20,21,22],banks=['0:1:0'])
        self.assertFalse(any(r['width']==4 for r in reference_fields(partial,bytes(rom))))

    def test_mirrored_ram_keeps_the_cpu_offset_evidence(self):
        row=self.row(destination=0x2000,cpu_destination=0x6000,entry_registers={'DI':0x6000,'DS':0})
        self.assertEqual(observations(row)[0]['codec_destination'],0x6000)
