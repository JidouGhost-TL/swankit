"""Synthetic operand geometry; generated words must retain values above 255."""
import ast
import importlib.util
from pathlib import Path
import tempfile
import unittest


path = Path(__file__).resolve().parents[1] / "generate-saves.py"
spec = importlib.util.spec_from_file_location("save_generation", path)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class ChecksumGeometry(unittest.TestCase):
    def test_word_generator_is_proven_from_reader_body(self):
        tree = ast.parse('def read_word(data, offset):\n return struct.unpack_from("<H", data, offset)[0]\n')
        functions = {f.name: f for f in tree.body}
        operand = ast.parse("(read_word(data, 2 * i) for i in range(3))", mode="eval").body
        self.assertEqual(generator.sum_unit(operand, functions), "word-le")
        tree.body[0].body[0].value.value.args[0].value = "<I"
        self.assertIsNone(generator.sum_unit(operand, functions))

    def test_unknown_generator_keeps_portable_arithmetic(self):
        operand = ast.parse("(value * 1000 for value in data)", mode="eval").body
        self.assertIsNone(generator.sum_unit(operand, {}))
        operand = ast.parse("data[2:8]", mode="eval").body
        self.assertEqual(generator.sum_unit(operand, {}), "byte")

    def test_unpack_requires_unsigned_little_endian_words(self):
        for fmt in ("<3H", "<%dH"):
            operand = ast.parse(f"struct.unpack({fmt!r}, data)", mode="eval").body
            self.assertEqual(generator.sum_unit(operand, {}), "word-le")
        for fmt in ("<I", ">H", "<h", "<BH"):
            operand = ast.parse(f"struct.unpack({fmt!r}, data)", mode="eval").body
            self.assertIsNone(generator.sum_unit(operand, {}))


class SourceLayoutSizes(unittest.TestCase):
    def parse(self, literal, radix=10):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "format.tsv"
            path.write_text("medium\toffset\tsize\tfield\tencoding\n"
                            f"cartridge\t100\t{literal}\tboard\tbytes\n")
            audit = []
            return generator.layouts(path, size_radix=radix, audit=audit), audit

    def test_default_decimal_does_not_infer_hexadecimal_provenance(self):
        for literal, expected in [("280", 280), ("40", 40), ("0xe1", 225), ("0x40", 64)]:
            fields, audit = self.parse(literal)
            self.assertEqual(fields[0]["size"], expected)
            self.assertEqual(audit[0]["parsed_size"], expected)
        fields, audit = self.parse("e1")
        self.assertEqual(fields, [])
        self.assertEqual(audit[0]["status"], "symbolic-or-unsupported")

    def test_only_explicit_source_convention_accepts_bare_hex_sizes(self):
        for literal, expected in [("e1", 225), ("40", 64), ("0xe1", 225), ("0x40", 64)]:
            fields, audit = self.parse(literal, 16)
            self.assertEqual(fields[0]["size"], expected)
            self.assertEqual(audit[0]["size_radix"], 16)
        for radix in [0, 8, None, True, 10.0]:
            with self.assertRaises(ValueError):
                self.parse("40", radix)

    def test_unknown_and_unbounded_sizes_are_reported_without_fallback(self):
        for literal in ["variable", "n*8", "", "0", "1048577"]:
            fields, audit = self.parse(literal)
            self.assertEqual(fields, [])
            self.assertNotEqual(audit[0]["status"], "fixed-field")

    def test_reviewed_merge_retains_all_existing_indices_and_defers_other_rows(self):
        def field(name, offset, size):
            return {"name": name, "medium": "cartridge", "offset": offset, "size": size}
        existing = [field("first", 1, 1), field("wide", 2, 40), field("owner", 300, 80)]
        source = [field("work", 0, 1), field("new", 10, 225), field("first", 1, 1),
                  field("wide", 2, 64), field("owner", 300, 128)]
        approved = [("new", "cartridge", 10), ("wide", "cartridge", 2)]
        actual = generator.merge_layout_fields(existing, source, approved)
        self.assertEqual(actual, [existing[0], source[3], existing[2], source[1]])
        self.assertEqual(existing[1]["size"], 40)
        self.assertEqual(actual[2]["size"], 80)
        with self.assertRaises(ValueError):
            generator.merge_layout_fields(existing, source, [("absent", "cartridge", 99)])
        with self.assertRaises(ValueError):
            generator.merge_layout_fields(existing, source + [source[0]], approved)
        with self.assertRaises(ValueError):
            generator.merge_layout_fields(existing + [existing[0]], source, approved)
        with self.assertRaises(ValueError):
            generator.merge_layout_fields(existing, source, approved + [approved[0]])
