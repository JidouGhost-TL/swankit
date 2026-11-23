#!/usr/bin/env python3
"""Exercise the exact published native Swankit CLI on an independent matching host."""

import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import stat
import struct
import tarfile
from urllib.parse import urlsplit
from urllib.request import HTTPSHandler, HTTPRedirectHandler, ProxyHandler, Request, build_opener
from zipfile import ZipFile


VERSION = "0.1.1"
RELEASE = f"https://github.com/JidouGhost-TL/swankit/releases/download/v{VERSION}"
SMOKE_SOURCE = "e31288eaf73f5f14eb796e2828e6339369045ed2"
SMOKE_URL = f"https://raw.githubusercontent.com/JidouGhost-TL/swankit/{SMOKE_SOURCE}/tools/release-smoke.py"
SMOKE_SHA = "a8293f4370db99d8e84ef0a5a491918261ac2f8dc4cde8d7aecb61f667406382"
PUBLIC_HOSTS = {"github.com", "raw.githubusercontent.com", "release-assets.githubusercontent.com",
                "objects.githubusercontent.com"}
TARGETS = {
    "linux-x86_64": ("Linux", "x86_64", "tar.gz", "swankit",
        "716380f15b3cc5fda91bac00008f9d945bc5b2983d8750cf39d1c955028d69e4",
        "984c63cce579493429f4f390c01851c6f75bae3516a143be82a2d4a64c107d01"),
    "windows-x86_64": ("Windows", "x86_64", "zip", "swankit.exe",
        "9e2de2afeae5c08c225d36c63d0ba7e34dae9af0d6b3a3f4617bb7792a3957f7",
        "58591a07ba4eb420827e0bb6f808d064bc84d1c67da18f86334c0e8591583063"),
    "macos-arm64": ("Darwin", "arm64", "tar.gz", "swankit",
        "f995de5bd75c2fe6fa237d7c575e40def124b83fe18534477602cb5d7d3ec7d9",
        "526d270bd08480c1626e542b1b30cc77a335a27421b3360dcdcd69a4b1c6eac0"),
    "macos-x86_64": ("Darwin", "x86_64", "tar.gz", "swankit",
        "303b16766c22b8237981d6b23f48cee68fcab9a7b403feef99023cc36a9d7aa3",
        "b921591820a8fa541bd42a0333112080df78e07134b4eb54136353eec2758da6"),
}
CHECKS = {
    "version", "help", "rom-info", "tile-png", "ips-apply", "save-hard-link-refusal",
    "directory-output-refusal", "save-decode", "save-edit", "save-constraint-refusal",
    "save-explicit-override", "save-repair", "asset-hard-link-refusal", "asset-force-replace",
    "asset-verify", "asset-extract", "asset-replace", "returned-map-reuse",
    "wrong-identity-refusal", "search", "wondergate-listeners", "wondergate-wire-reopen",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def public_url(url):
    parsed = urlsplit(url)
    require(parsed.scheme == "https" and parsed.hostname in PUBLIC_HOSTS and not parsed.username
            and not parsed.password and parsed.port in (None, 443), "unexpected download origin")


class PublicRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        public_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def obtain(url, limit):
    public_url(url)
    opener = build_opener(ProxyHandler({}), HTTPSHandler(), PublicRedirect())
    request = Request(url, headers={"User-Agent": "Swankit public release consumer"})
    with opener.open(request, timeout=30) as response:
        public_url(response.geturl())
        data = response.read(limit + 1)
    require(len(data) <= limit, "oversize download")
    return data


def target_files(target):
    _, _, suffix, binary, archive_sha, executable_sha = TARGETS[target]
    stem = f"swankit-{VERSION}-{target}"
    return f"{stem}.{suffix}", f"{stem}/{binary}", binary, archive_sha, executable_sha


def member_bytes(raw, archive, member):
    if archive.endswith(".zip"):
        with ZipFile(io.BytesIO(raw)) as package:
            info = [entry for entry in package.infolist() if entry.filename == member]
            require(len(info) == 1 and not info[0].is_dir()
                    and stat.S_IFMT(info[0].external_attr >> 16) in (0, stat.S_IFREG),
                    "ZIP lacks a unique regular executable")
            return package.read(info[0])
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as package:
        info = [entry for entry in package if entry.name == member]
        require(len(info) == 1 and info[0].isfile() and bool(info[0].mode & 0o111),
                "TAR lacks a unique regular executable")
        return package.extractfile(info[0]).read()


def host(target):
    def normalize(raw):
        return {"amd64": "x86_64", "x64": "x86_64", "aarch64": "arm64"}.get(raw.lower(), raw.lower())

    machine = normalize(platform.machine())
    system = platform.system()
    pointer_bits = struct.calcsize("P") * 8
    require((system, machine) == TARGETS[target][:2] and pointer_bits == 64,
            "runner OS, architecture or pointer width differs from target")
    if system == "Windows":
        require(normalize(os.environ.get("PROCESSOR_ARCHITECTURE", "")) == machine,
                "Windows process architecture differs from target")
    return {"system": system, "machine": machine, "pointer_bits": pointer_bits,
            "release": platform.release(), "python": platform.python_version()}


def pinned_bytes(raw, expected, label):
    require(sha(raw) == expected, f"{label} SHA-256 mismatch")
    return raw


def expected_checks(target):
    return CHECKS | {"unix-link-refusal"} if target == "windows-x86_64" else CHECKS


def output_proof(work):
    checks = work / "checks"
    tile = (checks / "tile.png").read_bytes()
    require(tile.startswith(b"\x89PNG\r\n\x1a\n") and tile[16:24] == bytes.fromhex("0000000800000008"),
            "tile output missing or dimensions differ")
    require((checks / "patched result").read_bytes() == b"aZc", "IPS control bytes differ")
    require((checks / "edited.eeprom").read_bytes() == bytes([165, 7]) + bytes(126), "edited save bytes differ")
    require((checks / "override.eeprom").read_bytes() == bytes([165, 10]) + bytes(126), "override save bytes differ")
    require((checks / "repaired.eeprom").read_bytes() == bytes([165]) + bytes(127), "repaired save bytes differ")
    edited = (checks / "edited.wsc").read_bytes()
    require(edited[256:272] == bytes(range(16)), "asset replacement bytes differ")
    require((checks / "again.wsc").read_bytes() == edited, "returned-map reuse output differs")
    for name in ("edited-map.json", "again-map.json"):
        mapping = json.loads((checks / name).read_bytes())
        require(mapping["rom"]["sha256"] == sha(edited), "returned map does not identify its edited ROM")
    require((checks / "extracted/record.bin").read_bytes() == bytes(16), "extracted asset bytes differ")
    require(not (checks / "refused.eeprom").exists() and not (checks / "refused-map").exists(),
            "refused operation created output")
    wire = (checks / "wire/wire.jsonl").read_bytes()
    rows = [json.loads(line) for line in wire.splitlines()]
    require(len(rows) == 2 and all(row["protocol"] == "HTTP" for row in rows), "loopback output differs")
    return {name: sha((checks / name).read_bytes()) for name in (
        "tile.png", "patched result", "edited.eeprom", "override.eeprom", "repaired.eeprom",
        "edited.wsc", "edited-map.json", "again.wsc", "again-map.json",
        "extracted/record.bin", "wire/wire.jsonl",
    )}


def check_list(target, checks):
    require(isinstance(checks, list) and all(isinstance(check, str) for check in checks)
            and len(checks) == len(set(checks)) and set(checks) == expected_checks(target),
            "missing, duplicated or unexpected runtime checks")


def identity(target):
    archive, _, _, archive_sha, executable_sha = target_files(target)
    return {"schema": "swankit-public-native-consumer/1", "status": "native-execution-pass",
            "acquisition": "anonymous-https", "version": VERSION, "target": target, "host": host(target),
            "archive": archive, "archive_sha256": archive_sha, "executable_sha256": executable_sha,
            "smoke_source": SMOKE_SOURCE, "smoke_sha256": SMOKE_SHA}


def verify_receipt(target, work):
    receipt = json.loads((work / "RECEIPT.json").read_text(encoding="utf-8"))
    require(isinstance(receipt, dict)
            and all(receipt.get(key) == value for key, value in identity(target).items()),
            "receipt does not qualify exact public native execution")
    check_list(target, receipt.get("checks"))
    archive, member, binary, archive_sha, executable_sha = target_files(target)
    raw = pinned_bytes((work / archive).read_bytes(), archive_sha, "public archive")
    embedded = pinned_bytes(member_bytes(raw, archive, member), executable_sha, "archive executable")
    require((work / binary).read_bytes() == embedded, "executed file differs from pinned member")
    pinned_bytes((work / "release-smoke.py").read_bytes(), SMOKE_SHA, "released smoke")
    require(receipt.get("outputs") == output_proof(work), "receipt runtime outputs differ")
    return receipt


def replay(target, work):
    receipt = identity(target)
    archive, member, binary, archive_sha, executable_sha = target_files(target)
    raw = pinned_bytes(obtain(f"{RELEASE}/{archive}", 5 * 1024 * 1024), archive_sha, "public archive")
    executable = pinned_bytes(member_bytes(raw, archive, member), executable_sha, "archive executable")
    smoke = pinned_bytes(obtain(SMOKE_URL, 128 * 1024), SMOKE_SHA, "released smoke")
    work.mkdir()
    (work / archive).write_bytes(raw)
    exe = work / binary
    exe.write_bytes(executable)
    if target != "windows-x86_64":
        exe.chmod(0o700)
    smoke_path = work / "release-smoke.py"
    smoke_path.write_bytes(smoke)
    spec = importlib.util.spec_from_file_location("published_smoke", smoke_path)
    published = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(published)
    require(published.run(exe, "--version").strip() == f"swankit {VERSION}", "unexpected binary version")
    checks = published.cli_smoke(exe, work / "checks")
    check_list(target, checks)
    receipt.update(checks=sorted(checks), outputs=output_proof(work))
    (work / "RECEIPT.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("replay", "verify"))
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--work", required=True, type=Path, help="fresh runtime directory")
    args = parser.parse_args()
    require(__debug__, "runtime assertions must be enabled")
    work = args.work.resolve()
    if args.command == "replay":
        replay(args.target, work)
    else:
        print(json.dumps(verify_receipt(args.target, work), sort_keys=True))


if __name__ == "__main__":
    main()
