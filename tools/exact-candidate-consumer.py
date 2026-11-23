#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Fetch and consume one independent exact Swankit candidate."""
import argparse
import ctypes
import errno
import hashlib
import http.client
from io import BytesIO
import json
import os
from pathlib import Path
import platform
import re
import stat
import struct
import subprocess
import tarfile
from urllib.parse import urlsplit
from zipfile import ZipFile


REPOSITORY = "JidouGhost-TL/swankit"
REPOSITORY_ID = 1418334066
SOURCE = "44255a000f3745d7ebb02cb36490750c537a356e"
RUN_ID = 45123657227
ARTIFACT_ID = 14794928412
ARTIFACT_SIZE = 17498649
TRANSPORT_SHA = "b4d3c0e9d69de00f725fbbaa7ea5857948a1971235437df7a3a4b1e1c1fb4fa4"
MANIFEST_SHA = "163376abe7cee6292b42c72ec738431c715379e44ab5953edf5575c5d7e6d1c9"
LOCK_SHA = "66a31f8919f6096f0add81f58bb51d8a60bf412c3eaa1b1e0629328baba9dc04"
TARGETS = {
    "windows-x86_64": ("Windows", "x86_64", "zip", "swankit.exe",
        "b62e6491fefd73ed9f65efa1ba39b90f61191d7101c6af010866b3748b9e9c3c",
        "70b4972cb744baedfb300647749942d0c61b7867d06ccd5dfd7f679b21ed43e7"),
    "macos-arm64": ("Darwin", "arm64", "tar.gz", "swankit",
        "1dca786c559830fb1d284e856fec9a944c40885d1878021659efae0ce67fc557",
        "90405ab3568ab44d87177965e278e80612d4cb3404fa80b904d3443c2d632250"),
    "macos-x86_64": ("Darwin", "x86_64", "tar.gz", "swankit",
        "35239414c28ed8f4babf4d33e9ef3d406201337b1ca2e80d124bb780b93df4b0",
        "5702d290931cb2fc283494787a0b56ad0f53c9e6e1c027e7260ed8899dbc7645"),
    "linux-x86_64": ("Linux", "x86_64", "tar.gz", "swankit",
        "c563d1d13a2ab6f14dba177c6d8d4a2707e71e83016de9ef51161862b2fc293a",
        "3c3333bd628a54b99254d7d34a2e09e68b82974e6857a23faf078ca5812fd4ff"),
}
API = {"Accept": "application/vnd.github+json", "User-Agent": "swankit-exact-candidate-consumer",
       "X-GitHub-Api-Version": "2022-11-28"}


def require(ready, code):
    if not ready:
        raise ValueError(code)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def https_get(host, route, headers, limit):
    connection = http.client.HTTPSConnection(host, timeout=45)
    try:
        connection.request("GET", route, headers=headers)
        with connection.getresponse() as response:
            redirect = response.getheader("Location") if response.status == 302 else None
            body = response.read(limit + 1) if response.status == 200 else b""
            require(len(body) <= limit, "response_limit")
            return response.status, redirect, body
    finally:
        connection.close()


def api_get(route, headers=API, limit=512 * 1024):
    return https_get("api.github.com", f"/repos/{REPOSITORY}{route}", headers, limit)


def storage_location(raw):
    url = urlsplit(raw or "")
    require(url.scheme == "https" and bool(url.path) and not url.fragment
            and not url.username and not url.password and url.port in (None, 443)
            and (url.hostname == "results-receiver.actions.githubusercontent.com"
                 or re.fullmatch(r"[a-z0-9]+\.blob\.core\.windows\.net", url.hostname or "")),
            "unexpected_storage_location")
    return url.hostname, url.path + ("?" + url.query if url.query else "")


def fetch(path, token):
    require(bool(token) and not path.exists(), "missing_token_or_existing_destination")
    status, _, data = api_get(f"/actions/runs/{RUN_ID}")
    require(status == 200, "run_metadata_status")
    run = json.loads(data)
    require(all(run.get(key) == value for key, value in {
        "id": RUN_ID, "event": "workflow_dispatch", "path": ".github/workflows/release-candidate.yml",
        "head_branch": "main", "head_sha": SOURCE, "conclusion": "success",
    }.items()) and run["repository"]["id"] == REPOSITORY_ID
            and run["head_repository"]["id"] == REPOSITORY_ID, "run_identity")

    route = f"/actions/artifacts/{ARTIFACT_ID}"
    status, _, data = api_get(route)
    require(status == 200, "artifact_metadata_status")
    artifact = json.loads(data)
    require(all(artifact.get(key) == value for key, value in {
        "id": ARTIFACT_ID, "name": "candidate-" + SOURCE, "expired": False,
        "digest": "sha256:" + TRANSPORT_SHA, "size_in_bytes": ARTIFACT_SIZE,
    }.items()) and artifact["workflow_run"] == {
        "id": RUN_ID, "repository_id": REPOSITORY_ID, "head_repository_id": REPOSITORY_ID,
        "head_branch": "main", "head_sha": SOURCE,
    }, "artifact_identity")

    status, redirect, _ = api_get(route + "/zip", {**API, "Authorization": "Bearer " + token}, limit=0)
    require(status == 302, "artifact_download_status")
    host, object_path = storage_location(redirect)
    status, _, payload = https_get(host, object_path, {}, ARTIFACT_SIZE)
    require(status == 200 and len(payload) == ARTIFACT_SIZE and sha(payload) == TRANSPORT_SHA,
            "transport_identity")
    with path.open("xb") as output:
        output.write(payload)


def zip_member(archive, name, maximum):
    matches = [entry for entry in archive.infolist() if entry.filename == name]
    require(len(matches) == 1, "zip_member_unique")
    item, = matches
    require(not item.is_dir() and 0 < item.file_size <= maximum
            and stat.S_IFMT(item.external_attr >> 16) in (0, stat.S_IFREG), "zip_member_type_or_size")
    return archive.read(item)


def package_executable(archive, filename, member):
    if filename.endswith(".zip"):
        with ZipFile(BytesIO(archive)) as packet:
            return zip_member(packet, member, 32 * 1024 * 1024)
    with tarfile.open(fileobj=BytesIO(archive), mode="r:gz") as packet:
        files = [entry for entry in packet if entry.name == member]
        require(len(files) == 1, "tar_member_unique")
        entry, = files
        require(entry.isfile() and 0 < entry.size <= 32 * 1024 * 1024 and entry.mode & 0o111,
                "tar_member_type_or_size")
        with packet.extractfile(entry) as data:
            return data.read()


def header(executable, target):
    if target == "windows-x86_64":
        pe = int.from_bytes(executable[60:64], "little")
        require(executable[:2] == b"MZ" and pe >= 64
                and executable[pe:pe + 6] == b"PE\0\0\x64\x86"
                and executable[pe + 24:pe + 26] == b"\x0b\x02", "expected_pe32_plus_amd64")
    elif target == "linux-x86_64":
        require(executable[:6] == b"\x7fELF\x02\x01" and executable[18:20] == b"\x3e\x00", "expected_elf64_amd64")
    else:
        cpu = 0x0100000C if target == "macos-arm64" else 0x01000007
        require(executable[:8] == b"\xcf\xfa\xed\xfe" + cpu.to_bytes(4, "little"), "expected_native_mach_o")


def packet_binary(path, target):
    require(path.stat().st_size == ARTIFACT_SIZE, "transport_size")
    raw = path.read_bytes()
    require(sha(raw) == TRANSPORT_SHA, "transport_hash")
    _, _, archive_suffix, binary, archive_sha, executable_sha = TARGETS[target]
    stem = f"swankit-0.1.1-{target}"
    filename = f"{stem}.{archive_suffix}"
    with ZipFile(BytesIO(raw)) as packet:
        manifest_raw = zip_member(packet, "candidate-manifest.json", 128 * 1024)
        require(sha(manifest_raw) == MANIFEST_SHA, "manifest_hash")
        manifest = json.loads(manifest_raw)
        require(all(manifest.get(key) == expected for key, expected in {
            "source_commit": SOURCE, "run_id": RUN_ID, "lock_sha256": LOCK_SHA,
            "schema": "swankit-candidate/2", "version": "0.1.1",
        }.items()) and manifest["files"].get(filename) == archive_sha, "manifest_binding")
        archive = zip_member(packet, "packet/" + filename, 8 * 1024 * 1024)
    require(sha(archive) == archive_sha, "nested_archive_hash")
    executable = package_executable(archive, filename, f"{stem}/{binary}")
    require(sha(executable) == executable_sha, "nested_executable_hash")
    header(executable, target)
    return binary, executable


def host(target):
    system, machine = TARGETS[target][:2]
    normalize = lambda value: {"amd64": "x86_64", "aarch64": "arm64"}.get(value.lower(), value.lower())
    require(platform.system() == system and normalize(platform.machine()) == machine
            and struct.calcsize("P") == 8, "native_host")
    if system == "Windows":
        require(normalize(os.environ.get("PROCESSOR_ARCHITECTURE", "")) == machine, "native_windows")
    elif system == "Darwin":
        value = ctypes.c_int()
        size = ctypes.c_size_t(ctypes.sizeof(value))
        rc = ctypes.CDLL(None, use_errno=True).sysctlbyname(
            b"sysctl.proc_translated", ctypes.byref(value), ctypes.byref(size), None, ctypes.c_size_t(0))
        require((rc == 0 and size.value == ctypes.sizeof(value) and value.value == 0)
                or (rc == -1 and ctypes.get_errno() == errno.ENOENT), "native_macos")


def runtime(executable, work):
    user = work / "fresh user"
    user.mkdir()
    command_dir = work / "consumer 空間"
    command_dir.mkdir()
    base_env = {"HOME": str(user), "USERPROFILE": str(user)}
    for name, child in {
        "PATH": "empty-path", "XDG_CONFIG_HOME": "config", "XDG_CACHE_HOME": "cache",
        "XDG_DATA_HOME": "data", "XDG_STATE_HOME": "state", "APPDATA": "roaming",
        "LOCALAPPDATA": "local", "TMPDIR": "temporary", "TEMP": "temporary", "TMP": "temporary",
    }.items():
        folder = user / child
        folder.mkdir(exist_ok=True)
        base_env[name] = str(folder)
    if os.name == "nt":
        base_env["SystemRoot"] = os.environ["SystemRoot"]

    def run(*args):
        return subprocess.run([executable, *map(str, args)], cwd=command_dir, env=base_env,
                              capture_output=True, timeout=25)

    version = run("--version")
    require(version.returncode == 0 and version.stdout.strip() == b"swankit 0.1.1", "version")
    expected = b"XYZXYZXY"
    tokens = b"\x10XYZ\x22\x00"
    header = bytearray(16)
    header[:4] = b"LZS2"
    header[4] = len(expected)
    header[8] = len(tokens)
    offset = 0x120
    source = bytearray(65536)
    source[offset:offset + len(header) + len(tokens)] = header + tokens

    def fixture(name, data):
        rom = command_dir / (name + " source.wsc")
        mapping = command_dir / (name + " map.json")
        rom.write_bytes(data)
        mapping.write_text(json.dumps({"schema": "swankit-asset-map/1",
            "rom": {"sha256": sha(data), "size": len(data)},
            "assets": [{"id": "record", "kind": "other", "offset": offset, "codec": "ws-lzs2"}]}), encoding="utf-8")
        return rom, mapping

    rom, mapping = fixture("positive", source)
    outcome = run("assets", "extract", rom.name, mapping.name, "decoded output", "--json")
    require(outcome.returncode == 0, "lzs2_decode")
    folder = command_dir / "decoded output"
    index = json.loads((folder / "index.json").read_bytes())
    entry, = index["assets"]
    require((folder / "record.bin").read_bytes() == expected and index["failures"] == 0
            and all(entry.get(key) == value for key, value in {
                "offset": offset, "stored_length": 22, "decoded_length": 8, "decoded_sha256": sha(expected),
            }.items()), "lzs2_extent_and_bytes")

    mutated = bytearray(source)
    mutated[offset + 8] += 1
    bad_rom, bad_map = fixture("excess", mutated)
    invalid = run("assets", "verify", bad_rom.name, bad_map.name, "--json")
    require(invalid.returncode != 0 and b"LZS2 consumed 6 of 7 token bytes" in invalid.stdout + invalid.stderr,
            "lzs2_consumption_refusal")
    (command_dir / "edited.bin").write_bytes(expected)
    forbidden = run("assets", "replace", rom.name, mapping.name, "--id", "record", "--from", "edited.bin",
                    "-o", "forbidden.wsc", "--force")
    require(forbidden.returncode != 0
            and forbidden.stderr.strip() == b"error: record: LZS2 assets are decode-only"
            and not (command_dir / "forbidden.wsc").exists() and rom.read_bytes() == source
            and bad_rom.read_bytes() == mutated, "lzs2_write_refusal")
    return {"stored_length": 22, "decoded_length": 8, "decoded_sha256": sha(expected),
            "overlap_copied": True, "excess_token_refused": True, "write_refused": True, "inputs_unchanged": True}


def replay(path, target, work):
    require(not any(os.environ.get(name) for name in ("SWANKIT_CANDIDATE_TOKEN", "GH_TOKEN", "GITHUB_TOKEN")),
            "credential_in_consumer_environment")
    host(target)
    filename, raw = packet_binary(path, target)
    work.mkdir()
    binary = work / filename
    binary.write_bytes(raw)
    if os.name != "nt":
        binary.chmod(0o700)
    proof = runtime(binary, work)
    require(binary.read_bytes() == raw, "executable_changed")
    return {"schema": 1, "status": "native_candidate_consumer_pass", "target": target, "source": SOURCE,
            "run": RUN_ID, "artifact": ARTIFACT_ID, "transport_sha256": TRANSPORT_SHA,
            "nested_archive_sha256": TARGETS[target][-2], "executable_sha256": sha(raw), "checks": proof}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("fetch", "replay"))
    parser.add_argument("--packet", type=Path, required=True)
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--work", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "fetch":
            fetch(args.packet, os.environ.pop("SWANKIT_CANDIDATE_TOKEN", ""))
        else:
            require(args.target and args.work, "consumer_arguments")
            receipt = replay(args.packet, args.target, args.work.resolve())
            print(json.dumps(receipt, separators=(",", ":")))
        return 0
    except Exception:
        # An HTTP error could otherwise print the one-use signed URL.
        print(json.dumps({"status": "candidate_consumer_refused", "stage": args.command}))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
