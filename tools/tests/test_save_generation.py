"""Synthetic operand geometry; generated words must retain values above 255."""
import ast
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
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


class ReviewedLayoutCommand(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        tool = self.root / "tools/generate-saves.py"
        tool.parent.mkdir()
        tool.write_bytes(path.read_bytes())
        self.tool = tool
        self.source = self.root / "source/sample"
        self.source.mkdir(parents=True)
        self.table = self.source / "format.tsv"
        self.table.write_text("medium\toffset\tsize\tfield\tencoding\n"
                              "cartridge\t10\te1\tnew board\tbytes\n"
                              "cartridge\t100\t40\told board\tbytes\n"
                              "cartridge\t200\t80\towner\tbytes\n")
        self.header = self.source / "save.h"
        self.header.write_text("typedef struct { uint8_t pad[16]; uint8_t new_board[225]; "
                               "uint8_t gap[15]; uint8_t old_board[64]; } Stored;\n")
        (self.source / "parse_save.py").write_text("def main():\n return None\nif __name__ == '__main__':\n main()\n")
        self.reference = {"id": "sample", "fields": generator.layouts(self.table),
                          "program": {"preserved": [1, 2]}, "records": [{"kept": True}],
                          "schema": "swankit-save-def/1"}
        self.refs = self.root / "references"
        self.refs.mkdir()
        (self.refs / "existing.json").write_text(json.dumps(self.reference))
        self.policy = {"version": 1, "sources": [{"definition_id": "sample",
                       "table_sha256": [self.sha(self.table)], "header_sha256": self.sha(self.header),
                       "size_radix": 16, "record_base": 0, "approved_count": 2,
                       "approved": [{"medium": "cartridge", "offset": 16, "packed_member": "new_board"},
                                    {"medium": "cartridge", "offset": 256, "packed_member": "old_board"}]}]}
        self.policy_path = self.root / "crates/ws-saves/source-layout-policies.json"
        self.policy_path.parent.mkdir(parents=True)
        self.policy_path.write_text(json.dumps(self.policy))

    @staticmethod
    def sha(file):
        return hashlib.sha256(file.read_bytes()).hexdigest()

    def run_repair(self, label="out", refs=None, only="sample"):
        output = self.root / label
        command = [sys.executable, str(self.tool), "--layout-only", "--source", str(self.source.parent),
                   "--output", str(output), "--policy", str(refs or self.refs), "--only", only,
                   "--layout-audit", str(self.root / (label + "-audit.json"))]
        result = subprocess.run(command, capture_output=True, text=True)
        return result, output

    def test_actual_repair_entrypoint_preserves_semantics_and_edit_indices(self):
        result, output = self.run_repair()
        self.assertEqual(result.returncode, 0, result.stderr)
        actual = json.loads((output / "existing.json").read_text())
        self.assertEqual({k: v for k, v in actual.items() if k != "fields"},
                         {k: v for k, v in self.reference.items() if k != "fields"})
        self.assertEqual([(f["name"], f["size"]) for f in actual["fields"]],
                         [("old board", 64), ("owner", 80), ("new board", 225)])
        audit = json.loads((self.root / "out-audit.json").read_text())
        self.assertEqual(audit[0]["source_rows"][-1]["deferred"][0]["size"], 128)
        repeated, second = self.run_repair("second", output)
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertEqual((output / "existing.json").read_bytes(), (second / "existing.json").read_bytes())

    def test_actual_command_refuses_table_header_and_binding_drift(self):
        original_table, original_header = self.table.read_text(), self.header.read_text()
        for case in ["table", "header", "both", "binding", "missing-binding"]:
            self.table.write_text(original_table + ("\n" if case in ["table", "both"] else ""))
            self.header.write_text(original_header + ("// drift\n" if case in ["header", "both"] else ""))
            policy = json.loads(json.dumps(self.policy))
            if case == "binding":
                policy["sources"][0]["approved"][0]["packed_member"] = "missing"
            if case == "missing-binding":
                policy["sources"][0]["approved"].pop()
            self.policy_path.write_text(json.dumps(policy))
            result, output = self.run_repair(case)
            self.assertNotEqual(result.returncode, 0, case)
            self.assertFalse((output / "existing.json").exists(), case)

    def test_bootstrap_cannot_silently_replace_reviewed_definition(self):
        output = self.root / "bootstrap"
        media = self.root / "media.json"
        media.write_text(json.dumps({"sample": []}))
        result = subprocess.run([sys.executable, str(self.tool), "--source", str(self.source.parent),
                                 "--output", str(output), "--media", str(media), "--records",
                                 str(self.root / "records.rs")], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("bootstrap cannot replace", result.stderr)
        self.assertFalse((output / "sample.json").exists())

    def test_unreviewed_decimal_source_retains_280(self):
        control = self.source.parent / "decimal-control"
        control.mkdir()
        table = control / "format.tsv"
        table.write_text("medium\toffset\tsize\tfield\tencoding\ncartridge\t0\t280\tpayload\tbytes\n")
        (control / "save.h").write_text("typedef struct { uint8_t payload[280]; } Payload;")
        reference = {"id": "decimal-control", "fields": generator.layouts(table), "program": {"keep": True}}
        (self.refs / "decimal.json").write_text(json.dumps(reference))
        result, output = self.run_repair("decimal", only="decimal-control")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads((output / "decimal.json").read_text()), reference)
