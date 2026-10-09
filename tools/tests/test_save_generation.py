"""Synthetic operand geometry; generated words must retain values above 255."""
import ast
import importlib.util
from pathlib import Path
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
