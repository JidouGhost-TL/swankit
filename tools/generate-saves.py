#!/usr/bin/env python3
"""Generate save layout descriptors and Rust-owned semantic programs from source tables.

Inputs are external research directories. Output contains grammar only: no saves,
paths, corpus populations or frozen expectations. Parsing never invokes Python.
"""
import argparse
import ast
import csv
import json
from pathlib import Path
import re
import subprocess


def node(value):
    if isinstance(value, ast.Constant) and isinstance(value.value, bytes):
        return {"t": "Bytes", "v": list(value.value)}
    if isinstance(value, ast.AST):
        result = {"t": type(value).__name__}
        for key, child in ast.iter_fields(value):
            if key not in ("ctx", "annotation", "returns", "type_comment", "decorator_list"):
                result[key] = node(child)
        return result
    if isinstance(value, list):
        return [node(item) for item in value]
    if isinstance(value, str):
        if re.search(r"/(?:home|mnt|var/tmp|tmp)/", value):
            return "<external>"
        return value
    return value


def trim(tree):
    """Discard executable module launch and all docstrings; retain grammar logic."""
    body = []
    for statement in tree.body:
        if isinstance(statement, (ast.Import, ast.ImportFrom)):
            continue
        if isinstance(statement, ast.Expr) and isinstance(statement.value, ast.Constant):
            continue
        if isinstance(statement, ast.If) and "__name__" in ast.unparse(statement.test):
            continue
        if isinstance(statement, ast.Expr) and isinstance(statement.value, ast.Call) and isinstance(statement.value.func, ast.Name) and statement.value.func.id == "signal":
            continue
        if isinstance(statement, ast.Assign) and isinstance(statement.value, ast.Call):
            call = statement.value
            if (isinstance(call.func, ast.Name) and call.func.id == "bytes" and call.args
                    and isinstance(call.args[0], (ast.List, ast.Tuple)) and len(call.args[0].elts) > 128):
                # ROM-derived tables are external inputs, never generated source bytes.
                key = ast.unparse(statement.targets[0])
                statement.value = ast.Call(func=ast.Name(id="input_table", ctx=ast.Load()),
                                           args=[ast.Constant(value=key)], keywords=[])
        body.append(statement)
    tree.body = body
    for item in ast.walk(tree):
        if hasattr(item, "body") and isinstance(item.body, list):
            item.body = [s for s in item.body if not (isinstance(s, ast.Expr) and isinstance(s.value, ast.Constant) and isinstance(s.value.value, str))]
    return tree


def layouts(path):
    fields = []
    with path.open() as source:
        rows = csv.DictReader(source, delimiter="\t")
        for row in rows:
            try:
                label = row.get("offset", "").strip()
                if not re.fullmatch(r"(?:0[xX])?[0-9a-fA-F]+", label):
                    continue
                offset = int(label, 16)
                size_label = row.get("size", "").strip()
                match = re.fullmatch(r"(0[xX][0-9a-fA-F]+|[0-9]+)(?:\s*\([^)]*\))?", size_label)
                if not match:
                    continue
                size = int(match[1], 16 if match[1].lower().startswith("0x") else 10)
            except ValueError:
                continue  # symbolic/conditional regions remain in the semantic program
            if offset < 0 or size <= 0 or offset + size > 1048576:
                continue
            encoding = row.get("encoding", "")
            scalar = {"u8": 1, "u16le": 2, "u32le": 4}
            field = {"name": row.get("field", ""), "medium": row.get("medium", ""),
                           "offset": offset, "size": size, "encoding": encoding,
                           "type": "unsigned" if scalar.get(encoding) == size else "bytes",
                           "meaning": (row.get("meaning") or ""), "confidence": (row.get("confidence") or "unknown"),
                           "evidence": (row.get("evidence") or "")}
            count = re.search(r"\[(\d+)\]", encoding)
            if count and int(count[1]) > 0 and size % int(count[1]) == 0:
                field["array"] = {"count": int(count[1]), "stride": size // int(count[1])}
            fields.append(field)
    return fields


def records(header):
    """Describe packed C records whose entire width is statically known."""
    text = re.sub(r"/\*.*?\*/|//[^\n]*", "", header, flags=re.S)
    sizes = {"uint8_t": 1, "int8_t": 1, "char": 1, "uint16_t": 2,
             "int16_t": 2, "uint32_t": 4, "int32_t": 4, "uint64_t": 8}
    for base, alias in re.findall(r"typedef\s+(\w+)\s+(\w+)\s*;", text):
        if base in sizes:
            sizes[alias] = sizes[base]
    found = []
    for body, name in re.findall(r"typedef\s+struct\s*(?:\w+\s*)?\{(.*?)\}\s*(\w+)\s*;", text, flags=re.S):
        width = 0
        members = []
        valid = True
        for declaration in body.split(";"):
            declaration = declaration.strip()
            if not declaration:
                continue
            match = re.fullmatch(r"(\w+)\s+(\w+)(?:\[(0x[\da-fA-F]+|\d+)\])?", declaration)
            if not match or match[1] not in sizes:
                valid = False
                break
            count = int(match[3], 0) if match[3] else 1
            member_size = sizes[match[1]] * count
            members.append({"name": match[2], "offset": width, "size": member_size,
                            "type": match[1], "count": count, "stride": sizes[match[1]]})
            width += member_size
        if valid and 0 < width <= 1048576:
            sizes[name] = width
            found.append({"name": name, "width": width, "fields": members})
    return found



def entry(tree):
    functions = {f.name: f for f in tree.body if isinstance(f, ast.FunctionDef)}
    main = functions["main"]
    mode = "program"
    for call in ast.walk(tree.body[-1]):
        if isinstance(call, ast.Call) and isinstance(call.func, ast.Name) and call.func.id == "main":
            if not call.args:
                mode = "sys-argv" if not main.args.args else "arguments"
            elif isinstance(call.args[0], ast.Subscript):
                mode = "arguments"
    if any(isinstance(n, ast.Call) and ast.unparse(n.func) == "argparse.ArgumentParser" for n in ast.walk(main)):
        decode = next(n.value.func.id for n in main.body if isinstance(n, ast.Assign)
                      and isinstance(n.value, ast.Call) and isinstance(n.value.func, ast.Name)
                      and n.value.func.id in functions)
        fix = next(n.func.id for n in ast.walk(main) if isinstance(n, ast.Call)
                   and isinstance(n.func, ast.Name) and n.func.id.startswith("fix"))
        return {"mode": "direct", "decode": decode, "fix": fix}
    return {"mode": mode}


def sum_unit(operand, functions):
    """Recognize byte iterables and generators of checked little-endian words."""
    if isinstance(operand, ast.Call) and ast.unparse(operand.func).startswith("struct.unpack"):
        return "word-le"
    if isinstance(operand, (ast.GeneratorExp, ast.ListComp)):
        element = operand.elt
        if isinstance(element, ast.Call) and isinstance(element.func, ast.Name):
            reader = functions.get(element.func.id)
            if reader is not None:
                body = [n for n in reader.body if not (isinstance(n, ast.Expr) and isinstance(n.value, ast.Constant))]
                if len(body) == 1 and isinstance(body[0], ast.Return):
                    value = body[0].value
                    if (isinstance(value, ast.Subscript) and isinstance(value.slice, ast.Constant)
                            and value.slice.value == 0 and isinstance(value.value, ast.Call)
                            and ast.unparse(value.value.func) == "struct.unpack_from"
                            and value.value.args and isinstance(value.value.args[0], ast.Constant)
                            and value.value.args[0].value == "<H"):
                        return "word-le"
        # Arbitrary arithmetic generators do not establish byte/word geometry.
        return None
    return "byte"

def generate(args):
    source = args.source.resolve(strict=True)
    output = args.output
    output.mkdir(parents=True, exist_ok=True)
    policies = {}
    checksum_policies = {}
    if args.policy:
        for path in sorted(args.policy.glob("*.json")):
            definition = json.loads(path.read_text())
            policies[definition["id"]] = definition.get("primitives", {})
            checksum_policies[definition["id"]] = {
                k: v for k, v in definition.get("checksums", {}).items() if "input" in v}
    media = json.loads(args.media.read_text())
    entries = []
    for directory in sorted(source.iterdir()):
        if not directory.is_dir() or directory.name.startswith("backup"):
            continue
        parser = directory / "publish/parse_save.py"
        header = directory / "publish/save.h"
        if not parser.exists():
            parser = directory / "parse_save.py"
        if not header.exists():
            header = directory / "save.h"
        table = directory / "format.tsv"
        if not parser.exists() or not table.exists():
            continue
        if args.only and directory.name not in args.only:
            continue
        # Both field tables and packed record declarations are generation inputs.
        h = header.read_text()
        tree = ast.parse(parser.read_text())
        launch = entry(tree)
        policy = policies.get(directory.name, {})
        for function in tree.body:
            if isinstance(function, ast.FunctionDef) and function.name in policy:
                function.body = [ast.Return(value=ast.Call(
                    func=ast.Name(id="definition_primitive", ctx=ast.Load()),
                    args=[ast.Constant(value=function.name)] +
                         [ast.Name(id=a.arg, ctx=ast.Load()) for a in function.args.args], keywords=[]))]
        crc_names = {"crc16_kermit": (0, 0, False), "crc_kermit": (0, 0, False),
                     "kermit": (0, 0, False), "crc16_kermit_swapped": (0, 0, True),
                     "crc16_x25": (65535, 65535, False)}
        checksums = {}
        functions = {f.name: f for f in tree.body if isinstance(f, ast.FunctionDef)}
        for function in tree.body:
            if isinstance(function, ast.FunctionDef) and function.name in crc_names:
                seed, xor_out, swap = crc_names[function.name]
                checksums[function.name] = {"library": "ws-save", "algorithm": "crc16",
                    "parameters": {"polynomial": 33800, "seed": seed, "xor_out": xor_out,
                                   "swap_bytes": swap, "seed_argument": 1}}
            if not isinstance(function, ast.FunctionDef):
                continue
            body = [n for n in function.body if not (isinstance(n, ast.Expr) and isinstance(n.value, ast.Constant))]
            # Lower explicit masked sums, preserving the source operand and its
            # bounds checks. Composite checks stay as portable semantic nodes.
            if len(body) == 1 and isinstance(body[0], ast.Return):
                value = body[0].value
                if (isinstance(value, ast.BinOp) and isinstance(value.op, ast.BitAnd)
                        and isinstance(value.right, ast.Constant) and value.right.value in (255, 65535, 4294967295)
                        and isinstance(value.left, ast.Call) and isinstance(value.left.func, ast.Name)
                        and value.left.func.id == "sum" and len(value.left.args) == 1):
                    operand = value.left.args[0]
                    unit = sum_unit(operand, functions)
                    if unit is None:
                        continue
                    checksums[function.name] = {"library": "ws-save", "algorithm": "fold", "parameters": {
                        "unit": unit, "fold": "sum", "bits": value.right.value.bit_length(), "seed": 0, "xor_out": 0, "offset_base": 0}}
                    body[0].value = ast.Call(func=ast.Name(id="definition_checksum", ctx=ast.Load()),
                                            args=[ast.Constant(value=function.name), operand], keywords=[])
                    function.body = body
        for function in tree.body:
            if isinstance(function, ast.FunctionDef) and function.name in checksum_policies.get(directory.name, {}):
                binding = checksum_policies[directory.name][function.name]
                geometry = binding["input"]
                operand = ast.Name(id=function.args.args[geometry["argument"]].arg, ctx=ast.Load())
                operand = ast.Subscript(value=operand, ctx=ast.Load(), slice=ast.Slice(
                    lower=ast.Constant(value=geometry["offset"]),
                    upper=ast.Constant(value=geometry["offset"] + geometry["size"]), step=None))
                function.body = [ast.Return(value=ast.Call(func=ast.Name(id="definition_checksum", ctx=ast.Load()),
                    args=[ast.Constant(value=function.name), operand], keywords=[]))]
                checksums[function.name] = binding
        fields = layouts(table)
        entries.append({"schema": "swankit-save-def/1", "id": directory.name,
                        "implementation": "generated", "media": media[directory.name],
                        "blocks": [{"field": i, "redundancy": "program"} for i, f in enumerate(fields)
                                   if any(s in f["encoding"].lower() for s in ("struct", "record", "slot", "copy"))],
                        "validity": {"policy": "program", "exclude": ["work_area", "current", "working_copy"]},
                        "checksums": checksums, "primitives": policy,
                        "entry": launch, "fields": fields, "records": records(h),
                        "program": node(trim(tree))})
    if not entries:
        raise ValueError("no complete save grammars")
    for definition in entries:
        slug = re.sub(r"[^a-z0-9]+", "-", definition["id"].lower()).strip("-")
        (output / (slug + ".json")).write_text(json.dumps(definition, ensure_ascii=False, separators=(",", ":")) + "\n")
    widths = sorted({r["width"] for e in entries for r in e["records"]})
    rust = "// Generated by tools/generate-saves.py; checked record extents from packed headers.\n"
    rust += "use formatkit_core::{Error, Result};\nuse formatkit_layout::FixedRecords;\n"
    rust += "pub fn record(bytes: &[u8], start: usize, width: usize, index: usize) -> Result<&[u8]> {\n    match width {\n"
    for width in widths:
        rust += f"        {width} => Ok(FixedRecords::<{width}>::new(start).record(bytes, index)?.as_slice()),\n"
    rust += '        _ => Err(Error::Malformed("unknown generated record width".into())),\n    }\n}\n'
    args.records.write_text(rust)
    if args.progress_log:
        stamp = subprocess.check_output(["date", "-Iminutes"], text=True).strip()
        with args.progress_log.open("a") as log:
            log.write(f"{stamp} | generation | {len(entries)} title grammars, {sum(len(e['fields']) for e in entries)} fixed fields, {sum(len(e['records']) for e in entries)} record descriptions | run parity\n")
    print(f"Generated {len(entries)} title grammars")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--source", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--only", action="append")
    p.add_argument("--policy", type=Path, help="existing definition directory with primitive bindings")
    p.add_argument("--media", type=Path, required=True, help="external medium geometry metadata")
    p.add_argument("--records", type=Path, required=True)
    p.add_argument("--progress-log", type=Path)
    generate(p.parse_args())


if __name__ == "__main__":
    main()
