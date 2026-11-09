#!/usr/bin/env python3
"""Generate save layout descriptors and Rust-owned semantic programs from source tables.

Inputs are external research directories. Output contains grammar only: no saves,
paths, corpus populations or frozen expectations. Parsing never invokes Python.
"""
import argparse
import ast
import csv
import hashlib
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


def layouts(path, *, size_radix=10, audit=None):
    """Read a source table using its explicitly established bare-size radix.

    Decimal remains the default; letters never imply hexadecimal provenance.
    Explicit 0x sizes retain their own radix. Optional audit rows describe
    omitted symbolic extents as well as accepted fixed fields.
    """
    if type(size_radix) is not int or size_radix not in (10, 16):
        raise ValueError("source size radix must be explicitly decimal or hexadecimal")
    fields = []
    with path.open() as source:
        rows = csv.DictReader(source, delimiter="\t")
        for line, row in enumerate(rows, 2):
            decision = {"line": line, "offset": row.get("offset", ""),
                        "size_literal": row.get("size", ""), "size_radix": size_radix,
                        "field": row.get("field", ""), "status": "symbolic-or-unsupported"}
            if audit is not None:
                audit.append(decision)
            try:
                label = row.get("offset", "").strip()
                if not re.fullmatch(r"(?:0[xX])?[0-9a-fA-F]+", label):
                    continue
                offset = int(label, 16)
                size_label = row.get("size", "").strip()
                bare = r"[0-9]+" if size_radix == 10 else r"[0-9a-fA-F]+"
                match = re.fullmatch(r"(0[xX][0-9a-fA-F]+|" + bare + r")(?:\s*\([^)]*\))?", size_label)
                if not match:
                    continue
                size = int(match[1], 16 if match[1].lower().startswith("0x") else size_radix)
            except ValueError:
                continue  # symbolic/conditional regions remain in the semantic program
            if offset < 0 or size <= 0 or offset + size > 1048576:
                decision["status"] = "out-of-range"
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
            decision.update(status="fixed-field", parsed_size=size)
    return fields


def merge_layout_fields(existing, source_fields, approved):
    """Apply explicitly reviewed source rows while preserving edit identifiers.

    Keys are (name, medium, offset). Existing indices never move; newly approved
    fields append in source order. Unapproved differences stay deferred.
    """
    key = lambda field: (field["name"], field["medium"], field["offset"])
    old_keys = [key(field) for field in existing]
    source_keys = [key(field) for field in source_fields]
    selected = list(approved)
    if len(set(old_keys)) != len(old_keys) or len(set(source_keys)) != len(source_keys):
        raise ValueError("ambiguous duplicate source or existing field identity")
    if len(set(selected)) != len(selected) or any(k not in source_keys for k in selected):
        raise ValueError("approved fields must uniquely identify fixed source rows")
    fields = list(existing)
    for field in source_fields:
        identity = key(field)
        if identity not in selected:
            continue
        if identity in old_keys:
            fields[old_keys.index(identity)] = field
        else:
            fields.append(field)
    return fields


def reviewed_layouts(table, header, existing, policies, audit, source_id=None):
    """Bind reviewed source conventions to exact table/header bytes and members."""
    table_hash = hashlib.sha256(table.read_bytes()).hexdigest()
    header_hash = hashlib.sha256(header.read_bytes()).hexdigest()
    identity_policies = [p for p in policies if p["definition_id"] == source_id]
    if identity_policies and (len(identity_policies) != 1 or table_hash not in identity_policies[0]["table_sha256"]):
        raise ValueError("table bytes differ from reviewed indexed source identity")
    matches = [p for p in policies if table_hash in p["table_sha256"]]
    if len(matches) > 1:
        raise ValueError("ambiguous reviewed layout source policy")
    if not matches:
        if any(header_hash == p["header_sha256"] for p in policies):
            raise ValueError("unreviewed table bytes for known packed layout header")
        fields = layouts(table, audit=audit)
        if existing is not None:
            keys = lambda rows: {(f["name"], f["medium"], f["offset"]): f for f in rows}
            if (len(keys(fields)) != len(fields) or len(keys(existing)) != len(existing)
                    or keys(fields) != keys(existing)):
                raise ValueError("unreviewed source differs from indexed layout fields")
            return list(existing)
        return fields
    policy = matches[0]
    if source_id is not None and policy["definition_id"] != source_id:
        raise ValueError("reviewed source identity does not match indexed definition")
    if header_hash != policy["header_sha256"]:
        raise ValueError("packed layout header differs from reviewed source pin")
    if existing is None or audit is None:
        raise ValueError("reviewed layout requires indexed field reference and external audit")
    fields = layouts(table, size_radix=policy["size_radix"], audit=audit)
    if len(policy["approved"]) != policy["approved_count"]:
        raise ValueError("reviewed source binding inventory is incomplete")
    members = [f for record in records(header.read_text()) for f in record["fields"]]
    approved = []
    for binding in policy["approved"]:
        selected = [f for f in fields if f["medium"] == binding["medium"] and f["offset"] == binding["offset"]]
        member = [f for f in members if f["name"] == binding["packed_member"]]
        if (len(selected) != 1 or len(member) != 1 or selected[0]["size"] != member[0]["size"]
                or selected[0]["offset"] != policy["record_base"] + member[0]["offset"]):
            raise ValueError("reviewed source field does not match packed member binding")
        approved.append((selected[0]["name"], selected[0]["medium"], selected[0]["offset"]))
    merged = merge_layout_fields(existing, fields, approved)
    key = lambda f: (f["name"], f["medium"], f["offset"])
    previous = {key(f): f for f in existing}
    audit.append({"status": "reviewed-source-policy", "table_sha256": table_hash,
                  "header_sha256": header_hash, "approved": approved,
                  "deferred": [f for f in fields if key(f) not in approved and previous.get(key(f)) != f]})
    return merged


def layout_source_policies():
    path = Path(__file__).resolve().parents[1] / "crates/ws-saves/source-layout-policies.json"
    document = json.loads(path.read_text())
    if document.get("version") != 1 or not isinstance(document.get("sources"), list):
        raise ValueError("unsupported reviewed source-layout policy version")
    return document["sources"]


def repair_layouts(args):
    """Repair only reviewed field metadata; canonical semantic data is retained."""
    if args.policy is None or args.layout_audit is None:
        raise ValueError("--layout-only requires existing canonical --policy and external --layout-audit")
    if args.layout_audit.resolve().is_relative_to(args.output.resolve()):
        raise ValueError("layout audit must be outside generated definition directory")
    policies = layout_source_policies()
    references = {}
    for path in args.policy.glob("*.json"):
        definition = json.loads(path.read_text())
        if definition["id"] in references:
            raise ValueError("duplicate indexed definition identity")
        references[definition["id"]] = (path.name, definition)
    repaired = []; audits = []
    for directory in sorted(args.source.resolve(strict=True).iterdir()):
        if not directory.is_dir() or (args.only and directory.name not in args.only):
            continue
        if directory.name not in references:
            raise ValueError("layout repair requires an existing indexed definition")
        table = directory / "format.tsv"
        header = directory / "publish/save.h"
        if not header.is_file():
            header = directory / "save.h"
        filename, reference = references[directory.name]
        audit = []
        fields = reviewed_layouts(table, header, reference["fields"], policies, audit, directory.name)
        candidate = dict(reference)
        candidate["fields"] = fields
        repaired.append((filename, candidate))
        audits.append({"id": directory.name, "source_rows": audit})
    if not repaired:
        raise ValueError("no existing indexed layouts selected")
    args.output.mkdir(parents=True, exist_ok=True)
    args.layout_audit.write_text(json.dumps(audits, ensure_ascii=False, indent=2) + "\n")
    for filename, definition in repaired:
        (args.output / filename).write_text(json.dumps(definition, ensure_ascii=False, separators=(",", ":")) + "\n")
    print(f"Repaired {len(repaired)} indexed layouts; semantic definitions unchanged")


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
        if not operand.args:
            return None
        fmt = operand.args[0]
        if isinstance(fmt, ast.BinOp) and isinstance(fmt.op, ast.Mod):
            fmt = fmt.left
        if isinstance(fmt, ast.Constant) and isinstance(fmt.value, str) and re.fullmatch(r"<(?:[0-9]+|%d)?H", fmt.value):
            return "word-le"
        return None
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
    if getattr(args, "layout_only", False):
        return repair_layouts(args)
    source = args.source.resolve(strict=True)
    output = args.output
    output.mkdir(parents=True, exist_ok=True)
    policies = {}
    checksum_policies = {}
    layout_policies = layout_source_policies()
    layout_audits = []
    audit_path = getattr(args, "layout_audit", None)
    if audit_path is not None and audit_path.resolve().is_relative_to(output.resolve()):
        raise ValueError("layout audit must be outside generated definition directory")
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
        table_hash = hashlib.sha256(table.read_bytes()).hexdigest()
        header_hash = hashlib.sha256(header.read_bytes()).hexdigest()
        if any(directory.name == p["definition_id"] or table_hash in p["table_sha256"]
               or header_hash == p["header_sha256"] for p in layout_policies):
            raise ValueError("bootstrap cannot replace a reviewed indexed definition; use --layout-only")
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
        layout_audit = [] if audit_path is not None else None
        fields = layouts(table, audit=layout_audit)
        if layout_audit is not None:
            layout_audits.append({"id": directory.name, "source_rows": layout_audit})
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
    if audit_path is not None:
        audit_path.write_text(json.dumps(layout_audits, ensure_ascii=False, indent=2) + "\n")
    for definition in entries:
        slug = re.sub(r"[^a-z0-9]+", "-", definition["id"].lower()).strip("-")
        destination = output / (slug + ".json")
        if destination.exists() and any(json.loads(destination.read_text()).get("id") == p["definition_id"] for p in layout_policies):
            raise ValueError("bootstrap cannot overwrite a protected indexed definition")
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
    p.add_argument("--media", type=Path, help="external medium geometry metadata (bootstrap only)")
    p.add_argument("--records", type=Path, help="generated record bindings (bootstrap only)")
    p.add_argument("--layout-only", action="store_true", help="repair reviewed fields against an existing canonical --policy; preserve semantic data")
    p.add_argument("--progress-log", type=Path)
    p.add_argument("--layout-audit", type=Path, help="external per-row reviewed source-layout audit")
    args = p.parse_args()
    if not args.layout_only and (args.media is None or args.records is None):
        p.error("bootstrap requires --media and --records; use --layout-only for indexed field repair")
    generate(args)


if __name__ == "__main__":
    main()
