#!/usr/bin/env python3
"""Create synthetic saves and check the public CLI and installed Python API.

Requires an installed swankit package and the swankit executable. Writes only
to a new output directory. Definitions must already exist beside this script.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

import swankit


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def stamp():
    return datetime.now(timezone.utc).isoformat()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def record(value, generation):
    # Independent fixture construction: twelve reserved bytes are preserved.
    return bytes([0xA5, value, generation, 0x11] + [0] * 10
                 + [0x22, (0xA5 + value + generation + 0x11 + 0x22) % 256])


def decoded_record(value, generation, checksum_ok=True, signature_valid=True):
    return dict(value=value, generation=generation, signature_valid=signature_valid,
                range_ok=value <= 9, checksum_ok=checksum_ok,
                valid=signature_valid and value <= 9 and checksum_ok)


class Trial:
    def __init__(self, cli, out):
        self.cli = cli
        self.out = out
        self.receipts = []
        self.diffs = {}

    def save_receipt(self, receipt):
        self.receipts.append(receipt)
        (self.out / "COMMANDS.json").write_text(
            json.dumps(self.receipts, indent=2) + "\n", encoding="utf-8")

    def command(self, label, args, refuse=False):
        argv = [self.cli, "save", *map(str, args)]
        started = stamp()
        result = subprocess.run(argv, capture_output=True, text=True)
        (self.out / (label + ".stdout")).write_text(result.stdout, encoding="utf-8")
        (self.out / (label + ".stderr")).write_text(result.stderr, encoding="utf-8")
        self.save_receipt(dict(label=label, interface="CLI", argv=argv,
                               started=started, finished=stamp(), exit=result.returncode))
        require(result.returncode == (1 if refuse else 0),
                f"{label}: unexpected exit {result.returncode}: {result.stderr}")
        if not refuse:
            return json.loads(result.stdout)
        require(bool(result.stderr.strip()), label + ": refusal had no diagnostic")

    def api(self, label, definition, method, data, refuse=False, **kwargs):
        receipt = dict(label=label, interface="Python", definition=definition.id,
                       method=method, input_sha256=digest(data), arguments=kwargs,
                       started=stamp())
        try:
            result = getattr(definition, method)(data, **kwargs)
        except swankit.SwankitError as error:
            receipt.update(finished=stamp(), error=type(error).__name__, diagnostic=str(error))
            self.save_receipt(receipt)
            require(refuse, f"{label}: unexpected {error}")
            return None
        receipt.update(finished=stamp(), result_hex=result.hex() if isinstance(result, bytes)
                       else None, result=result if not isinstance(result, bytes) else None)
        self.save_receipt(receipt)
        require(not refuse, label + ": unsafe operation unexpectedly succeeded")
        return result

    def diff(self, label, before, after, offsets):
        require(len(before) == len(after), label + ": length changed")
        diff = [dict(offset=i, before=a, after=b)
                for i, (a, b) in enumerate(zip(before, after)) if a != b]
        require([row["offset"] for row in diff] == offsets,
                f"{label}: unexpected byte differences: {diff}")
        self.diffs[label] = diff

    def run(self, kind, definition_path):
        text = definition_path.read_text(encoding="utf-8")
        definition_json = json.loads(text)
        # Catch truncated hand-written ASTs even where runtime metadata is unused.
        def full_ast(node):
            if isinstance(node, dict):
                require(node.get("t") not in ("Import", "ImportFrom"), "unsupported import node")
                if node.get("t") == "FunctionDef":
                    require(node.get("type_params") == [], "FunctionDef requires type_params: []")
                for child in node.values():
                    full_ast(child)
            elif isinstance(node, list):
                for child in node:
                    full_ast(child)
        full_ast(definition_json["program"])
        definition = swankit.SaveDefinition(text)
        multiple = kind == "redundant"
        first, second = record(3, 2), record(2, 1)
        damaged_second = second[:-1] + bytes([second[-1] ^ 0x80])
        original = first + damaged_second if multiple else first
        image = self.out / (kind + ".eep")
        image.write_bytes(original)
        expected_tree = (dict(copies=[decoded_record(3, 2), decoded_record(2, 1, False)], selected=0)
                         if multiple else decoded_record(3, 2))
        tree = self.command(kind + "-decode", ["decode", definition_path, image, "--json"])
        require(tree == expected_tree, kind + ": CLI decode differs from fixture oracle")
        require(self.api(kind + "-py-decode", definition, "decode", original) == tree,
                kind + ": Python decode differs from CLI")
        require(self.api(kind + "-py-verify", definition, "verify", original) is not None,
                kind + ": no usable record")

        edited_path = self.out / (kind + "-edited.eep")
        self.command(kind + "-set", ["set", definition_path, image, "--field", "0",
                                     "--value", "7", "--output", edited_path, "--json"])
        expected_edit = record(7, 2) + second if multiple else record(7, 2)
        edited = edited_path.read_bytes()
        require(edited == expected_edit, kind + ": CLI set failed full-byte oracle")
        require(self.api(kind + "-py-set", definition, "set_field", original, field=0, value=7)
                == expected_edit, kind + ": Python set failed full-byte oracle")
        self.diff(kind + "-set", original, edited, [1, 15, 31] if multiple else [1, 15])
        after = self.api(kind + "-py-edited-decode", definition, "decode", edited)
        expected_after = (dict(copies=[decoded_record(7, 2), decoded_record(2, 1)], selected=0)
                          if multiple else decoded_record(7, 2))
        require(after == expected_after, kind + ": edited semantic tree differs")
        require(self.api(kind + "-py-edited-verify", definition, "verify", edited) is not None,
                kind + ": edited bytes have no usable record")

        damaged = original if multiple else first[:-1] + bytes([first[-1] ^ 0x80])
        damaged_path = self.out / (kind + "-damaged.eep")
        damaged_path.write_bytes(damaged)
        fixed_path = self.out / (kind + "-fixed.eep")
        self.command(kind + "-fix", ["fix", definition_path, damaged_path,
                                     "--output", fixed_path, "--json"])
        expected_fix = first + second if multiple else first
        require(fixed_path.read_bytes() == expected_fix, kind + ": CLI repair changed wrong bytes")
        require(self.api(kind + "-py-fix", definition, "fix", damaged) == expected_fix,
                kind + ": Python repair changed wrong bytes")
        self.diff(kind + "-fix", damaged, expected_fix, [31] if multiple else [15])

        # Without repair the edited record's checksum is stale, so no record is
        # usable: the CLI's --no-repair and Python's repair=False both refuse.
        self.api(kind + "-py-norepair-refusal", definition, "set_field", original,
                 field=0, value=7, repair=False, refuse=True)
        norepair_path = self.out / (kind + "-norepair-refused.eep")
        self.command(kind + "-norepair-refusal", ["set", definition_path, image, "--field", "0",
                     "--value", "7", "--no-repair", "--output", norepair_path, "--json"],
                     refuse=True)
        require(not norepair_path.exists(), kind + ": refused --no-repair edit created output")
        raw = self.api(kind + "-py-raw-set", definition, "set_field", original,
                       field=0, value=7, repair=False, skip_validation=True)
        self.diff(kind + "-raw-set", original, raw, [1])
        require(self.api(kind + "-py-raw-fix", definition, "fix", raw) == expected_edit,
                kind + ": explicit raw edit then repair differs")

        if multiple:
            fallback = bytearray(first + second)
            fallback[0] = 0
            fallback_path = self.out / "redundant-fallback.eep"
            fallback_path.write_bytes(fallback)
            fallback_tree = self.command("redundant-fallback-decode",
                                         ["decode", definition_path, fallback_path, "--json"])
            require(fallback_tree == dict(copies=[decoded_record(3, 2, False, False),
                                                  decoded_record(2, 1)], selected=1),
                    "redundant: valid second copy was not selected")
            require(self.api("redundant-fallback-verify", definition, "verify", bytes(fallback))
                    is not None, "redundant: valid sibling was refused")

        invalid = bytearray(expected_fix)
        invalid[0] = 0
        if multiple:
            invalid[16] = 0
        invalid = bytes(invalid)
        invalid_path = self.out / (kind + "-invalid.eep")
        invalid_path.write_bytes(invalid)
        require(self.api(kind + "-py-invalid-verify", definition, "verify", invalid) is None,
                kind + ": invalid signatures recognized as usable")
        for action in ("fix", "set", "set-norepair", "set-unchecked"):
            # Every edit path refuses an input with no usable record before
            # editing or repairing it, including --no-repair and --skip-validation.
            flags = {"set": [], "set-norepair": ["--no-repair"],
                     "set-unchecked": ["--skip-validation"]}.get(action)
            options = ["--field", "0", "--value", "7", *flags] if flags is not None else []
            method = "set_field" if flags is not None else "fix"
            kwargs = dict(field=0, value=7) if flags is not None else {}
            if action == "set-norepair":
                kwargs["repair"] = False
            if action == "set-unchecked":
                kwargs["skip_validation"] = True
            self.api(kind + "-py-refuse-" + action, definition, method, invalid,
                     refuse=True, **kwargs)
            fresh = self.out / (kind + "-refused-" + action + ".eep")
            verb = "fix" if flags is None else "set"
            self.command(kind + "-refuse-" + action, [verb, definition_path, invalid_path,
                         *options, "--output", fresh, "--json"], refuse=True)
            require(not fresh.exists(), kind + ": refused operation created output")
            sentinel = self.out / (kind + "-sentinel-" + action + ".eep")
            sentinel.write_bytes(b"preserve this destination")
            self.command(kind + "-force-refuse-" + action, [verb, definition_path, invalid_path,
                         *options, "--output", sentinel, "--force", "--json"], refuse=True)
            require(sentinel.read_bytes() == b"preserve this destination",
                    kind + ": force bypassed semantic refusal")
        self.api(kind + "-py-range-refusal", definition, "set_field", original,
                 field=0, value=256, refuse=True)
        range_path = self.out / (kind + "-range-refused.eep")
        self.command(kind + "-range-refusal", ["set", definition_path, image, "--field", "0",
                     "--value", "256", "--output", range_path, "--json"], refuse=True)
        require(not range_path.exists(), kind + ": out-of-range edit created output")
        require(image.read_bytes() == original and invalid_path.read_bytes() == invalid
                and damaged_path.read_bytes() == damaged, kind + ": an input was modified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, help="path or PATH name of swankit")
    parser.add_argument("--out", required=True, type=Path, help="new directory; must not exist")
    parser.add_argument("--definitions", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    cli = shutil.which(args.cli)
    require(cli is not None, "swankit executable not found: " + args.cli)
    definitions = {kind: (args.definitions / (kind + ".json")).resolve()
                   for kind in ("single", "redundant")}
    for path in definitions.values():
        require(path.is_file(), "missing example definition: " + str(path))
    args.out.mkdir(parents=True, exist_ok=False)
    trial = Trial(str(Path(cli).resolve()), args.out.resolve())
    for kind, path in definitions.items():
        trial.run(kind, path)
    result = dict(status="PASS", cli_sha256=digest(Path(cli).read_bytes()),
                  python_package=swankit.__file__, definitions={kind: digest(path.read_bytes())
                  for kind, path in definitions.items()}, byte_diffs=trial.diffs,
                  calls=len(trial.receipts))
    (args.out / "RESULT.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print("PASS: single and redundant examples; exact bytes and safe refusals checked")
    print(args.out / "RESULT.json")


if __name__ == "__main__":
    main()
