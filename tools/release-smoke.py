#!/usr/bin/env python3
"""Exercise installed distributions with caller-owned synthetic inputs only."""
import argparse
import hashlib
import json
import os
import platform
import queue
import socket
from pathlib import Path
import subprocess
import shutil
import sys
import tarfile
import threading


def run(*args, success=True, env=None):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True, env=env)
    if (result.returncode == 0) != success:
        raise ValueError("unexpected command result: " + str(args[1:3]) + "\n" + result.stderr)
    return result.stdout


def save_definition():
    constant = lambda v: {"t": "Constant", "value": v}
    data = {"t": "Name", "id": "data"}
    field = lambda n: {"t": "Subscript", "value": data, "slice": constant(n)}
    function = lambda n, args, body: {"t": "FunctionDef", "name": n,
                  "args": {"args": [{"arg": a} for a in args], "defaults": []}, "body": body}
    return {"schema": "swankit-save-def/2", "id": "synthetic", "implementation": "generated",
            "media": [{"kind": "cartridge-eeprom", "sizes": [128]}],
            "fields": [{"name": "setting", "medium": "cart EEPROM", "offset": 1, "size": 1,
                        "type": "unsigned", "encoding": "u8", "meaning": "Synthetic setting",
                        "confidence": "synthetic", "evidence": "Synthetic definition",
                        "constraint": {"kind": "unsigned-range", "minimum": 0, "maximum": 9}}],
            "records": [], "blocks": [], "checksums": {}, "primitives": {},
            "validity": {"policy": "program", "exclude": ["working_copy"]},
            "entry": {"mode": "direct", "decode": "decode_image", "fix": "fix_image"},
            "program": {"t": "Module", "type_ignores": [], "body": [
                function("main", [], [{"t": "Return", "value": constant(None)}]),
                function("decode_image", ["data", "name"], [{"t": "Return", "value": {"t": "Dict",
                    "keys": [constant("valid"), constant("setting")],
                    "values": [{"t": "Compare", "left": field(0), "ops": [{"t": "Eq"}],
                                "comparators": [constant(165)]}, field(1)]}}]),
                function("fix_image", ["data"], [{"t": "Assign", "targets": [field(0)], "value": constant(165)},
                                                   {"t": "Return", "value": data}])]}}


def smoke_server(executable, work):
    server = subprocess.Popen(
        [str(executable), "wg-server", "--data", str(work / "server-data"),
         "--wire", str(work / "wire"), "--http-port", "0", "--bpl-port", "0"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    lines = queue.Queue()
    reader = threading.Thread(target=lambda: lines.put(server.stdout.readline()))
    reader.start()
    try:
        line = lines.get(timeout=15).strip()
        parts = line.split()
        if (len(parts) != 3 or parts[0] != "WonderGate"
                or not parts[1].startswith("HTTP:") or not parts[2].startswith("BPL:")):
            raise ValueError("WonderGate server failed to report both listeners")
        for field in parts[1:]:
            host, port = field.split(":", 1)[1].rsplit(":", 1)
            with socket.create_connection((host, int(port)), timeout=5):
                pass
    finally:
        if server.poll() is None:
            server.terminate()
        server.wait(timeout=10)
        reader.join(timeout=10)
        error = server.stderr.read()
        server.stdout.close()
        server.stderr.close()
        if error:
            print(error, file=sys.stderr)


def cli_smoke(executable, work):
    work.mkdir()
    assert run(executable, "--version").startswith("swankit ")
    run(executable, "--help")
    checks = ["version", "help"]
    rom = bytearray(65536)
    rom[-16:] = bytes([234, 0, 0, 0, 240, 0, 1, 0, 2, 1, 0, 0, 0, 1, 0, 0])
    rom[-2:] = (sum(rom[:-2]) & 65535).to_bytes(2, "little")
    original = bytes(rom)
    image = work / "sample cart 測試.wsc"
    image.write_bytes(original)
    assert json.loads(run(executable, "info", image, "--json"))["checksum_ok"]
    run(executable, "tiles", image, "--format", "2bpp", "--length", "16", "--columns", "1",
        "--output", work / "tile.png")
    assert (work / "tile.png").read_bytes().startswith(bytes([137, 80, 78, 71]))
    checks += ["rom-info", "tile-png"]
    patch = work / "synthetic.ips"
    source = work / "patch source"
    patched = work / "patched result"
    patch.write_bytes(b"PATCH" + bytes([0, 0, 1, 0, 1]) + b"ZEOF")
    source.write_bytes(b"abc")
    run(executable, "patch", "apply", patch, source, "--output", patched)
    assert patched.read_bytes() == b"aZc" and source.read_bytes() == b"abc"
    checks.append("ips-apply")
    definition = work / "save.json"
    definition.write_text(json.dumps(save_definition()))
    save = work / "input.eeprom"
    save.write_bytes(bytes([165, 1]) + bytes(126))
    run(executable, "save", "decode", definition, save, "--json")
    edited = work / "edited.eeprom"
    run(executable, "save", "set", definition, save, "--field", "0", "--value", "7", "--output", edited)
    assert edited.read_bytes()[1] == 7 and save.read_bytes()[1] == 1
    alias = work / "save alias"
    os.link(save, alias)
    run(executable, "save", "set", definition, save, "--field", "0", "--value", "7",
        "--output", alias, "--force", success=False)
    run(executable, "save", "set", definition, save, "--field", "0", "--value", "7",
        "--output", work, "--force", success=False)
    assert alias.read_bytes() == save.read_bytes() == bytes([165, 1]) + bytes(126)
    checks += ["save-hard-link-refusal", "directory-output-refusal"]
    run(executable, "save", "set", definition, save, "--field", "0", "--value", "10", "--output",
        work / "refused.eeprom", success=False)
    assert not (work / "refused.eeprom").exists()
    run(executable, "save", "set", definition, save, "--field", "0", "--value", "10", "--skip-validation",
        "--output", work / "override.eeprom")
    assert (work / "override.eeprom").read_bytes()[1] == 10
    damaged = work / "damaged.eeprom"
    damaged.write_bytes(bytes(128))
    run(executable, "save", "fix", definition, damaged, "--output", work / "repaired.eeprom")
    assert (work / "repaired.eeprom").read_bytes()[0] == 165
    checks += ["save-decode", "save-edit", "save-constraint-refusal", "save-explicit-override", "save-repair"]
    asset_map = work / "map.json"
    asset_map.write_text(json.dumps({"schema": "swankit-asset-map/1",
        "rom": {"sha256": hashlib.sha256(original).hexdigest(), "size": len(original)},
        "assets": [{"id": "record", "kind": "data-table", "offset": 256, "length": 16}]}))
    run(executable, "assets", "verify", image, asset_map, "--json")
    run(executable, "assets", "extract", image, asset_map, work / "extracted", "--json")
    replacement = work / "replacement.raw"
    replacement.write_bytes(bytes(range(16)))
    run(executable, "assets", "replace", image, asset_map, "--id", "record", "--from", replacement,
        "-o", work / "edited.wsc", "--map-out", work / "edited-map.json")
    assert (work / "edited.wsc").read_bytes()[256:272] == bytes(range(16))
    run(executable, "assets", "verify", work / "edited.wsc", work / "edited-map.json", "--json")
    run(executable, "assets", "replace", work / "edited.wsc", work / "edited-map.json", "--id", "record",
        "--from", replacement, "-o", work / "again.wsc", "--map-out", work / "again-map.json")
    cart_alias = work / "cartridge alias"
    os.link(image, cart_alias)
    run(executable, "assets", "replace", image, asset_map, "--id", "record", "--from", replacement,
        "-o", cart_alias, "--map-out", work / "refused-map", "--force", success=False)
    assert not (work / "refused-map").exists()
    run(executable, "assets", "replace", image, asset_map, "--id", "record", "--from", replacement,
        "-o", work / "edited.wsc", "--map-out", work / "edited-map.json", "--force")
    assert image.read_bytes() == cart_alias.read_bytes() == original
    checks += ["asset-hard-link-refusal", "asset-force-replace"]
    run(executable, "assets", "verify", work / "edited.wsc", asset_map, "--json", success=False)
    run(executable, "assets", "search", image, "--json")
    checks += ["asset-verify", "asset-extract", "asset-replace", "returned-map-reuse", "wrong-identity-refusal", "search"]
    smoke_server(executable, work)
    checks.append("wondergate-listeners")
    if os.name == "nt":
        link = subprocess.run([str(executable), "link", "stdio", "exec:unused"],
                              capture_output=True, text=True)
        assert link.returncode and "link relay requires a Unix system" in link.stderr
        checks.append("unix-link-refusal")
    return checks


def smoke(packet, output):
    assert shutil.which("rustc") is None and shutil.which("cargo") is None, "packet smoke requires compiler-free runtime"
    work = output.parent / "packet-smoke"
    work.mkdir()
    temporary = work / "temporary"
    temporary.mkdir()
    os.environ["TMPDIR"] = str(temporary)
    archives = list(packet.glob("*-linux-x86_64.tar.gz"))
    wheels = list(packet.glob("*.whl"))
    assert len(archives) == len(wheels) == 1
    with tarfile.open(archives[0], "r:gz") as archive:
        members = archive.getmembers()
        assert all(m.isfile() and not Path(m.name).is_absolute() and ".." not in Path(m.name).parts
                   for m in members)
        archive.extractall(work / "cli", filter="data")
    executable = next((work / "cli").glob("*/swankit"))
    executable.chmod(0o755)
    checks = cli_smoke(executable, work / "checks")
    install = work / "installed"
    run(sys.executable, "-m", "pip", "install", "--no-index", "--no-deps", "--no-cache-dir", "--target", install, wheels[0])
    environment = dict(os.environ, PYTHONPATH=str(install))
    run(sys.executable, "-c", "import swankit; im = swankit.tiles(bytes(16), '2bpp', columns=1); "
        "assert (im.width, im.height, len(im.rgba)) == (8, 8, 256)", env=environment)
    checks.append("installed-wheel-core")
    result = {"schema": "swankit-packet-smoke/1", "status": "PASS", "checks": checks,
              "platform": {"system": platform.system(), "machine": platform.machine(),
                           "release": platform.release(), "libc": platform.libc_ver()},
              "wheel_sha256": hashlib.sha256(wheels[0].read_bytes()).hexdigest(),
              "cli_sha256": hashlib.sha256(executable.read_bytes()).hexdigest()}
    output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--packet", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    smoke(args.packet, args.output)
