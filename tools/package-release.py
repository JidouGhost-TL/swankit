#!/usr/bin/env python3
"""Assemble existing Linux artifacts and complete offline guides; never build."""
import argparse
import base64
import csv
import gzip
import hashlib
import io
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import tomllib
from urllib.parse import unquote, urlsplit
import zipfile

NOTICES = ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE")
GENERATED_GUIDE_DIRS = {"target", "build", "dist", "node_modules", "__pycache__", "release-licenses"}
_LICENSE_SPEC = importlib.util.spec_from_file_location("dependency_licenses", Path(__file__).with_name("collect-dependency-licenses.py"))
DEPENDENCY_LICENSES = importlib.util.module_from_spec(_LICENSE_SPEC)
_LICENSE_SPEC.loader.exec_module(DEPENDENCY_LICENSES)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def command(*args, cwd=None):
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True, text=True).stdout


def elf_audit(path):
    header = path.read_bytes()[:20]
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 62:
        raise ValueError("release requires a little-endian ELF64 x86_64 artifact")
    versions = command("readelf", "--version-info", str(path))
    needed = sorted(set(re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", versions)),
                    key=lambda value: tuple(map(int, value.split("."))))
    if not needed:
        raise ValueError("no GLIBC symbol requirements found; audit this build separately")
    return {"format": "ELF64 x86_64", "glibc_symbol_versions": needed,
            "glibc_symbol_minimum": needed[-1], "version_info": versions}


def private_guide_path(name):
    return any(part.startswith(".") or part in GENERATED_GUIDE_DIRS for part in Path(name).parts)


def local_links(payload):
    for name, data in payload.items():
        if not name.endswith(".md"):
            continue
        for target in re.findall(r"\]\(([^\s)]+)(?:\s+[^)]*)?\)", data.decode()):
            link = urlsplit(target.strip("<>"))
            if link.scheme or link.netloc or not link.path:
                continue
            decoded_path = unquote(link.path)
            if decoded_path.startswith("/"):
                raise ValueError(f"absolute guide link: {name}: {target}")
            parts = list(Path(name).parent.parts)
            for part in Path(decoded_path).parts:
                if part not in (".", "..") and private_guide_path(part):
                    raise ValueError(f"hidden or generated offline guide link: {name}: {target}")
                if part == "..":
                    if not parts:
                        raise ValueError(f"guide link escapes packet: {name}: {target}")
                    parts.pop()
                elif part != ".":
                    parts.append(part)
            yield name, target, "/".join(parts)


def guide_payload(root):
    names = {"README.md", "CONTRIBUTING.md", *NOTICES}
    names.update(p.relative_to(root).as_posix() for p in (root / "docs").rglob("*")
                 if p.is_file() and p.suffix in (".md", ".json"))
    names.update(p.relative_to(root).as_posix() for p in (root / "crates").rglob("README.md"))
    names = {name for name in names if not private_guide_path(name)}
    def read_guide(name):
        if private_guide_path(name):
            raise ValueError(f"hidden or generated offline guide target: {name}")
        path = root / name
        if (not path.is_file() or path.is_symlink()
                or not path.resolve().is_relative_to(root.resolve())):
            raise ValueError(f"missing or unsafe offline guide target: {name}")
        return path.read_bytes()

    payload = {name: read_guide(name) for name in sorted(names)}
    # Follow local documentation links transitively, including linked definition
    # JSON and nested crate guides, instead of guessing a fixed guide inventory.
    while True:
        missing = {resolved for _, _, resolved in local_links(payload) if resolved not in payload}
        if not missing:
            break
        for name in sorted(missing):
            payload[name] = read_guide(name)
    validate_links(payload)
    return payload


def validate_links(payload):
    for name, target, resolved in local_links(payload):
        if resolved not in payload:
            raise ValueError(f"missing offline guide target: {name}: {target}")


def wheel_audit(path, root, scratch, dependency_bundle=None):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("duplicate wheel members")
        for name in names:
            if name.startswith("/") or ".." in Path(name).parts:
                raise ValueError("unsafe wheel path")
        records = [n for n in names if n.endswith(".dist-info/RECORD")]
        if len(records) != 1:
            raise ValueError("wheel must have one RECORD")
        covered = set()
        for name, digest, size in csv.reader(io.StringIO(archive.read(records[0]).decode())):
            data = archive.read(name)
            if name in covered:
                raise ValueError(f"duplicate wheel RECORD row: {name}")
            covered.add(name)
            if name == records[0] and not digest and not size:
                continue
            expected = "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).decode().rstrip("=")
            if digest != expected or size != str(len(data)):
                raise ValueError(f"wheel RECORD mismatch: {name}")
        if covered != set(names):
            raise ValueError("wheel RECORD does not cover all members")
        for notice in NOTICES:
            matches = [n for n in names if n.endswith(".dist-info/licenses/" + notice)]
            if len(matches) != 1 or archive.read(matches[0]) != (root / notice).read_bytes():
                raise ValueError(f"wheel notice mismatch: {notice}")
        if dependency_bundle is not None:
            expected = {p.relative_to(dependency_bundle).as_posix(): p.read_bytes()
                        for p in dependency_bundle.rglob("*") if p.is_file()}
            prefix = next(n for n in names if n.endswith(".dist-info/RECORD")).rsplit("/", 1)[0] + "/licenses/release-licenses/"
            actual = {n[len(prefix):]: archive.read(n) for n in names if n.startswith(prefix)}
            if actual != expected:
                raise ValueError("wheel dependency notices differ or are missing")
        metadata = [n for n in names if n.endswith(".dist-info/METADATA")]
        tags = [n for n in names if n.endswith(".dist-info/WHEEL")]
        extensions = [n for n in names if n.endswith(".so")]
        if len(metadata) != 1 or len(tags) != 1 or len(extensions) != 1:
            raise ValueError("expected one Python metadata set and native extension")
        extension = scratch / "extension.so"
        extension.write_bytes(archive.read(extensions[0]))
        return {"metadata": archive.read(metadata[0]).decode(),
                "wheel_metadata": archive.read(tags[0]).decode(),
                "extension_sha256": sha(extension.read_bytes()), "elf": elf_audit(extension)}


def archive_bytes(payload, prefix):
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, data in sorted(payload.items()):
                member = tarfile.TarInfo(prefix + "/" + name)
                member.size = len(data)
                member.mode = 0o755 if name == "swankit" else 0o644
                archive.addfile(member, io.BytesIO(data))
    return buf.getvalue()


def package(root, cli, wheel, output, source_commit):
    root, cli, wheel, output = (p.resolve() for p in (root, cli, wheel, output))
    if output.exists() or output.is_relative_to(root):
        raise ValueError("output must be a new directory outside the source checkout")
    actual_commit = command("git", "rev-parse", "HEAD", cwd=root).strip()
    if source_commit != actual_commit:
        raise ValueError("--source-commit must equal the full source checkout HEAD")
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if command(str(cli), "--version").strip() != f"swankit {version}":
        raise ValueError("CLI version differs from source package")
    payload = guide_payload(root)
    dependency_bundle = root / "crates/swankit-py/release-licenses"
    dependency_inventory = DEPENDENCY_LICENSES.verify_bundle(dependency_bundle, sha((root / "Cargo.lock").read_bytes()))
    for path in dependency_bundle.rglob("*"):
        if path.is_file():
            payload["third-party-licenses/" + path.relative_to(dependency_bundle).as_posix()] = path.read_bytes()
    cli_elf = elf_audit(cli)
    with tempfile.TemporaryDirectory(prefix="swankit-package-") as temporary:
        wheel_info = wheel_audit(wheel, root, Path(temporary), dependency_bundle)
    if f"Version: {version}\n" not in wheel_info["metadata"]:
        raise ValueError("wheel version differs from source package")
    if "cp311-abi3-" not in wheel_info["wheel_metadata"] or "x86_64" not in wheel_info["wheel_metadata"]:
        raise ValueError("expected a Python 3.11+ abi3 x86_64 wheel")
    source_names = command("git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", cwd=root).split("\0")
    source_hashes = {name: sha((root / name).read_bytes()) for name in sorted(set(source_names))
                     if name and (root / name).is_file()}
    binding = {"source_commit": actual_commit,
               "source_status": command("git", "status", "--porcelain", cwd=root).splitlines(),
               "source_files_sha256": source_hashes,
               "cli_sha256": sha(cli.read_bytes()), "wheel_sha256": sha(wheel.read_bytes()),
               "cli_elf": cli_elf, "wheel": wheel_info,
               "scope": "Supplied artifacts and source hashes; no reproducible-build or platform-matrix claim."}
    binding["dependency_licenses"] = {"inventory_sha256": sha((dependency_bundle / "DEPENDENCIES.json").read_bytes()),
                                      "scope": dependency_inventory["scope"],
                                      "packages": len(dependency_inventory["packages"])}
    binding_bytes = (json.dumps(binding, indent=2, sort_keys=True) + "\n").encode()
    prefix = f"swankit-{version}-linux-x86_64"
    tarball = prefix + ".tar.gz"
    # Keep the source README at its normal path so all of its relative links work.
    packet_readme = f"""# Offline swankit {version} packet

CLI: Linux x86_64, glibc >= {cli_elf['glibc_symbol_minimum']} by ELF symbol requirements.
Wheel: Python 3.11+ abi3, Linux x86_64; glibc >= {wheel_info['elf']['glibc_symbol_minimum']} by ELF symbol requirements.
Symbol requirements are a lower bound, not execution tests on that oldest runtime.
Other OS/architecture combinations and a wider Python/glibc matrix are untested.

```sh
tar -xzf {tarball}
./{prefix}/swankit --version
./{prefix}/swankit tiles --help
python3 -m pip install --no-index --no-deps ./{wheel.name}
```

No Rust compiler is needed to use these existing artifacts. The optional
`to_pil` helper needs a separately supplied Pillow installation; core RGBA
operations do not. No ROMs, saves or per-ROM maps are included.

For a packet-only synthetic operation, create a 16-byte tile:

```sh
python3 -c 'from pathlib import Path; Path("tile.bin").write_bytes(bytes([128, 0]) * 8)'
./{prefix}/swankit tiles tile.bin --format 2bpp --length 16 --columns 1 --output tile.png
python3 -c 'import swankit; print(swankit.tiles(bytes(16), "2bpp", columns=1).width)'
```

Use fresh output paths. Read [the user quickstart](README.md), [CLI guide](docs/cli.md),
[Python guide](docs/python.md), [asset maps](docs/asset-maps.md) and
[save definitions](docs/save-definitions.md) before editing your own files.
All linked local guides, JSON schemas, crate guides and license notices are
included here and inside the CLI archive. BUILD-SOURCE.json binds the source
commit, actual source file hashes, artifacts and ELF requirements; SHA256SUMS
covers packet files. This packet performs no publication.
""".encode()
    payload["BUILD-SOURCE.json"] = binding_bytes
    payload["OFFLINE.md"] = packet_readme
    validate_links(payload)
    output.mkdir(parents=True)
    for name, data in payload.items():
        dest = output / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
    (output / wheel.name).write_bytes(wheel.read_bytes())
    (output / tarball).write_bytes(archive_bytes({**payload, "swankit": cli.read_bytes()}, prefix))
    sums = [(p.relative_to(output).as_posix(), sha(p.read_bytes())) for p in sorted(output.rglob("*")) if p.is_file()]
    (output / "SHA256SUMS").write_text("".join(f"{digest}  {name}\n" for name, digest in sums))
    return {"archive": tarball, "wheel": wheel.name, "files": len(sums) + 1,
            "source_commit": actual_commit, "cli_glibc_symbol_minimum": cli_elf["glibc_symbol_minimum"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--wheel", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-commit", required=True)
    args = parser.parse_args()
    print(json.dumps(package(Path(__file__).resolve().parents[1], args.cli, args.wheel,
                             args.output, args.source_commit), indent=2))


if __name__ == "__main__":
    main()
