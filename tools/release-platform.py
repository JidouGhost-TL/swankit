#!/usr/bin/env python3
"""Build and inspect native CLI release archives and their smoke receipts."""
import argparse
import gzip
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import platform
import subprocess
import tarfile
import tomllib
import zipfile

TARGETS = {
    "x86_64-pc-windows-msvc": ("windows-x86_64", "zip", "swankit.exe", "Windows", ("AMD64", "x86_64")),
    "aarch64-apple-darwin": ("macos-arm64", "tar.gz", "swankit", "Darwin", ("arm64", "aarch64")),
    "x86_64-apple-darwin": ("macos-x86_64", "tar.gz", "swankit", "Darwin", ("x86_64",)),
}
CHECKS = {
    "version", "help", "rom-info", "tile-png", "ips-apply", "save-hard-link-refusal",
    "directory-output-refusal", "save-decode", "save-edit", "save-constraint-refusal",
    "save-explicit-override", "save-repair", "asset-hard-link-refusal", "asset-force-replace",
    "asset-verify", "asset-extract", "asset-replace", "returned-map-reuse",
    "wrong-identity-refusal", "search", "wondergate-listeners",
}
NOTICES = ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE")


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def archive_spec(version, target):
    stem, extension, binary, _, _ = TARGETS[target]
    prefix = f"swankit-{version}-{stem}"
    return f"{prefix}.{extension}", prefix, binary


def member_name(name, size, prefix, binary, notices, files):
    if not name.startswith(prefix + "/"):
        raise ValueError("unexpected native archive root")
    leaf = name[len(prefix) + 1:]
    if leaf in files or leaf not in set(notices) | {binary}:
        raise ValueError("unexpected or duplicate native archive member")
    if leaf == binary:
        if not 0 < size <= 64 * 1024 * 1024:
            raise ValueError("invalid native executable size")
    elif size != len(notices[leaf]):
        raise ValueError("native archive notice size differs")
    return leaf


def read_archive(path, prefix, binary, notices):
    files = {}
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            for member in archive.infolist():
                kind = (member.external_attr >> 16) & 0o170000
                if kind not in {0, 0o100000} or member.is_dir():
                    raise ValueError("non-regular native ZIP member")
                leaf = member_name(member.filename, member.file_size, prefix, binary, notices, files)
                files[leaf] = archive.read(member)
    else:
        with tarfile.open(path, mode="r:gz") as archive:
            for member in archive:
                if not member.isfile():
                    raise ValueError("unsafe native tar member")
                leaf = member_name(member.name, member.size, prefix, binary, notices, files)
                if leaf == binary and not member.mode & 0o111:
                    raise ValueError("native tar binary is not executable")
                files[leaf] = archive.extractfile(member).read()
    if files.keys() != set(notices) | {binary} or any(files[name] != raw for name, raw in notices.items()):
        raise ValueError("missing or modified native archive notices")
    return files[binary]


def notices_from(root, licenses):
    notices = {name: (root / name).read_bytes() for name in NOTICES}
    notices.update({"third-party-licenses/" + path.relative_to(licenses).as_posix(): path.read_bytes()
                    for path in licenses.rglob("*") if path.is_file()})
    if "third-party-licenses/DEPENDENCIES.json" not in notices:
        raise ValueError("missing dependency license inventory")
    return notices


def write_archive(path, prefix, binary, payload):
    if path.suffix == ".zip":
        with zipfile.ZipFile(path, mode="x", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, raw in sorted(payload.items()):
                member = zipfile.ZipInfo(prefix + "/" + name)
                member.create_system = 3
                member.external_attr = 0o100644 << 16
                archive.writestr(member, raw, compress_type=zipfile.ZIP_DEFLATED)
    else:
        with path.open("xb") as output, gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                for name, raw in sorted(payload.items()):
                    member = tarfile.TarInfo(prefix + "/" + name)
                    member.mode = 0o755 if name == binary else 0o644
                    member.size = len(raw)
                    archive.addfile(member, io.BytesIO(raw))


def validate_receipt(receipt, target, source_commit, lock_sha256, version, packet, notices):
    name, prefix, binary = archive_spec(version, target)
    _, _, _, system, machines = TARGETS[target]
    if (receipt.get("schema") != "swankit-native-cli/1" or receipt.get("target") != target
            or receipt.get("source_commit") != source_commit or receipt.get("lock_sha256") != lock_sha256
            or receipt.get("version") != version or receipt.get("archive") != name
            or receipt.get("status") != "PASS" or set(receipt.get("checks", [])) != CHECKS):
        raise ValueError("native CLI source or smoke receipt mismatch")
    host = receipt.get("tested_platform", {})
    if host.get("system") != system or host.get("machine") not in machines or not host.get("os_version"):
        raise ValueError("unexpected native runner platform")
    path = packet / name
    if path.is_symlink() or digest(path.read_bytes()) != receipt.get("archive_sha256"):
        raise ValueError("native archive digest mismatch")
    executable = read_archive(path, prefix, binary, notices)
    if digest(executable) != receipt.get("executable_sha256"):
        raise ValueError("native executable digest mismatch")
    validate_executable(executable, target)


def validate_executable(executable, target):
    if target == "x86_64-pc-windows-msvc":
        pe = int.from_bytes(executable[60:64], "little")
        if (len(executable) < 64 or executable[:2] != b"MZ" or pe < 64
                or executable[pe:pe + 6] != b"PE\0\0\x64\x86"):
            raise ValueError("expected a Windows x86-64 PE executable")
    else:
        cpu = 0x0100000C if target == "aarch64-apple-darwin" else 0x01000007
        if executable[:8] != b"\xcf\xfa\xed\xfe" + cpu.to_bytes(4, "little"):
            raise ValueError("unexpected macOS Mach-O architecture")


def build(root, target, executable, licenses, output, scratch):
    run = lambda *args: subprocess.check_output(args, cwd=root, text=True).strip()
    if run("git", "status", "--porcelain"):
        raise ValueError("native build source must be clean")
    source = run("git", "rev-parse", "HEAD")
    rust_host = next(line.partition(": ")[2] for line in run("rustc", "-Vv").splitlines()
                     if line.startswith("host: "))
    if rust_host != target:
        raise ValueError("native runner toolchain does not match requested target")
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    lock = digest((root / "Cargo.lock").read_bytes())
    name, prefix, binary = archive_spec(version, target)
    output.mkdir(parents=True)
    notices = notices_from(root, licenses)
    archive = output / name
    write_archive(archive, prefix, binary, {**notices, binary: executable.read_bytes()})
    scratch.mkdir(parents=True)
    extracted = scratch / binary
    extracted.write_bytes(read_archive(archive, prefix, binary, notices))
    extracted.chmod(0o755)
    spec = importlib.util.spec_from_file_location("release_smoke", root / "tools/release-smoke.py")
    smoke = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(smoke)
    if smoke.run(extracted, "--version").strip() != f"swankit {version}":
        raise ValueError("native executable version does not match source")
    checks = smoke.cli_smoke(extracted, scratch / "checks")
    result = {
        "schema": "swankit-native-cli/1", "status": "PASS", "target": target, "version": version,
        "source_commit": source, "lock_sha256": lock, "archive": name,
        "archive_sha256": digest(archive.read_bytes()), "executable_sha256": digest(extracted.read_bytes()),
        "checks": checks, "tested_platform": {
            "system": platform.system(), "machine": platform.machine(),
            "os_version": platform.mac_ver()[0] if platform.system() == "Darwin" else platform.version(),
        },
    }
    validate_receipt(result, target, source, lock, version, output, notices)
    (output / "receipt.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--licenses", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    args = parser.parse_args()
    build(Path(__file__).resolve().parents[1], args.target, args.executable.resolve(),
          args.licenses.resolve(), args.output.resolve(), args.scratch.resolve())
