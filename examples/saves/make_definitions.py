#!/usr/bin/env python3
"""Rebuild the teaching definitions using only Python's standard library.

The strings below describe bounded save programs. They are serialized, never
executed by this generator. The runtime provides bytes/sum without imports.
"""

import ast
import json
from pathlib import Path


RECORD = """
def record(data):
    signature_valid = data[0] == 165
    range_ok = data[1] <= 9
    checksum_ok = data[15] == sum(data[:15]) % 256
    return {'value': data[1], 'generation': data[2],
            'signature_valid': signature_valid, 'range_ok': range_ok,
            'checksum_ok': checksum_ok,
            'valid': signature_valid and range_ok and checksum_ok}

def protect(data):
    return data[:15] + bytes([sum(data[:15]) % 256])
"""

SINGLE = RECORD + """
def decode_image(data, name):
    return record(data)

def repair_image(data):
    return protect(data)
"""

REDUNDANT = RECORD + """
def decode_image(data, name):
    first = record(data[:16])
    second = record(data[16:])
    selected = None
    if first['valid']:
        selected = 0
    if second['valid']:
        if selected == None or second['generation'] > first['generation']:
            selected = 1
    return {'copies': [first, second], 'selected': selected}

def repair_image(data):
    return protect(data[:16]) + protect(data[16:])
"""


def serialize(node):
    """Use the v1 structural AST, including metadata absent in Python 3.11.

    Python AST contexts, annotations, decorators and location metadata are not
    v1 properties. Reject unexpected syntax rather than dropping operations.
    """
    if isinstance(node, ast.Constant) and isinstance(node.value, bytes):
        return {"t": "Bytes", "v": list(node.value)}
    if isinstance(node, ast.AST):
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            raise ValueError("save programs cannot contain imports")
        ignored = {"ctx", "annotation", "returns", "decorator_list", "type_comment"}
        result = {"t": type(node).__name__}
        result.update((key, serialize(value)) for key, value in ast.iter_fields(node)
                      if key not in ignored)
        if isinstance(node, ast.FunctionDef):
            result.setdefault("type_params", [])
        return result
    if isinstance(node, list):
        return [serialize(item) for item in node]
    return node


def field(name, offset):
    return {"name": name, "medium": "cartridge EEPROM", "offset": offset,
            "size": 1, "type": "unsigned", "encoding": "u8",
            "meaning": "Teaching setting: integers 0 through 9.",
            "confidence": "synthetic", "evidence": "Defined by this example."}


def definition(kind):
    multiple = kind == "redundant"
    return {
        "schema": "swankit-save-def/1", "id": "example-" + kind,
        "implementation": "generated",
        "media": [{"kind": "cartridge-eeprom", "sizes": [32 if multiple else 16]}],
        "fields": [field("first_value", 1), field("second_value", 17)] if multiple
                  else [field("value", 1)],
        "records": [], "blocks": [{"field": 0, "redundancy": "program"}] if multiple else [],
        "checksums": {}, "primitives": {},
        "validity": {"policy": "program", "exclude": []},
        "entry": {"mode": "direct", "decode": "decode_image", "fix": "repair_image"},
        "program": serialize(ast.parse(REDUNDANT if multiple else SINGLE)),
    }


if __name__ == "__main__":
    for kind in ("single", "redundant"):
        path = Path(__file__).with_name(kind + ".json")
        path.write_text(json.dumps(definition(kind), indent=2) + "\n", encoding="utf-8")
        print(path.name)
