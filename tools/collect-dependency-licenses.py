#!/usr/bin/env python3
"""Preserve license evidence for the complete resolved Cargo package inventory."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import tarfile
import tomllib

MARKERS = ("LICENSE", "LICENCE", "NOTICE", "COPYING", "COPYRIGHT", "UNLICENSE")


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def relative(name):
    if not name or PurePosixPath(name).is_absolute() or "\\" in name or any(x in ("", ".", "..") for x in name.split("/")):
        raise ValueError("unsafe notice path")


def verify_bundle(root, lock_sha256=None):
    inventory_path = root / "DEPENDENCIES.json"
    if inventory_path.is_symlink():
        raise ValueError("symbolic notice inventory")
    inventory = json.loads(inventory_path.read_bytes())
    if inventory.get("schema") != "swankit-dependency-licenses/1" or inventory.get("scope") != "all-resolved-packages-including-build-dev-and-target-specific":
        raise ValueError("unknown dependency notice inventory")
    if lock_sha256 is not None and inventory.get("lock_sha256") != lock_sha256:
        raise ValueError("dependency notice lock binding differs")
    expected = {"DEPENDENCIES.json"}
    seen = set()
    if not inventory.get("packages"):
        raise ValueError("empty dependency notice inventory")
    for package in inventory["packages"]:
        key = (package["name"], package["version"])
        if key in seen or not package.get("licenses"):
            raise ValueError("duplicate or unlicensed package inventory")
        seen.add(key)
        for item in package["licenses"]:
            name = item["bundle_path"]
            relative(name)
            target = root / name
            if name in expected or target.is_symlink() or not target.is_file():
                raise ValueError("missing or duplicate dependency notice")
            raw = target.read_bytes()
            if len(raw) != item["bytes"] or sha(raw) != item["sha256"]:
                raise ValueError("dependency notice bytes changed")
            expected.add(name)
    actual = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError("symbolic dependency notice")
        if path.is_file():
            actual.add(path.relative_to(root).as_posix())
    if actual != expected:
        raise ValueError("dependency notice inventory coverage differs")
    return inventory


def collect(metadata, lock_raw, cargo_home, output):
    lock = tomllib.loads(lock_raw.decode())
    pins = {(p["name"], p["version"], p.get("source")): p for p in lock["package"]}
    inventory = {"schema": "swankit-dependency-licenses/1",
                 "scope": "all-resolved-packages-including-build-dev-and-target-specific",
                 "lock_sha256": sha(lock_raw), "packages": []}
    payload = {}
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        name, version, source = package["name"], package["version"], package.get("source")
        pin = pins.get((name, version, source))
        if pin is None:
            raise ValueError("resolved package absent from lockfile: " + name)
        root = Path(package["manifest_path"]).parent.resolve()
        archive_members = None
        checksum = None
        if source is not None:
            if source != "registry+https://github.com/rust-lang/crates.io-index" or not pin.get("checksum"):
                raise ValueError("unqualified dependency source: " + name)
            archives = list((cargo_home / "registry/cache").glob("*/" + name + "-" + version + ".crate"))
            if len(archives) != 1:
                raise ValueError("missing or ambiguous published archive: " + name)
            raw_archive = archives[0].read_bytes()
            checksum = pin["checksum"]
            if sha(raw_archive) != checksum:
                raise ValueError("published archive checksum differs: " + name)
            prefix = name + "-" + version + "/"
            archive_members = {}
            with tarfile.open(fileobj=io.BytesIO(raw_archive), mode="r:gz") as archive:
                for member in archive:
                    if member.isfile() and member.name.startswith(prefix):
                        relative_name = member.name[len(prefix):]
                        relative(relative_name)
                        if relative_name in archive_members:
                            raise ValueError("duplicate published archive member")
                        archive_members[relative_name] = archive.extractfile(member).read()
        license_file = package.get("license_file")
        selected = set()
        for path in root.rglob("*"):
            if path == output or output in path.parents:
                continue
            rel = path.relative_to(root)
            if path.is_file() and any(part.upper().startswith(MARKERS) for part in rel.parts):
                selected.add(rel.as_posix())
        if license_file:
            explicit = Path(license_file)
            if explicit.is_absolute():
                explicit = explicit.relative_to(root)
            relative(explicit.as_posix())
            selected.add(explicit.as_posix())
        if not selected or not (package.get("license") or license_file):
            raise ValueError("missing declared license evidence: " + name)
        record = {"name": name, "version": version, "declared_license": package.get("license"),
                  "source": source or "workspace", "registry_checksum": checksum, "licenses": []}
        for rel in sorted(selected):
            path = root / rel
            if path.is_symlink() or not path.is_file():
                raise ValueError("missing or symbolic declared license: " + name)
            raw = path.read_bytes()
            if archive_members is not None and archive_members.get(rel) != raw:
                raise ValueError("license differs from published archive: " + name)
            destination = name + "-" + version + "/" + rel
            relative(destination)
            payload[destination] = raw
            record["licenses"].append({"source_path": rel, "bundle_path": destination,
                                      "bytes": len(raw), "sha256": sha(raw)})
        inventory["packages"].append(record)
    payload["DEPENDENCIES.json"] = (json.dumps(inventory, indent=2, sort_keys=True) + "\n").encode()
    # Validate all inputs before writing; matching generated bundles are reused.
    if output.exists():
        verify_bundle(output, inventory["lock_sha256"])
        if {p.relative_to(output).as_posix(): p.read_bytes() for p in output.rglob("*") if p.is_file()} != payload:
            raise ValueError("generated notices differ; use a fresh source checkout")
    else:
        output.mkdir(parents=True)
        for name, raw in payload.items():
            path = output / name
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(raw)
    verify_bundle(output, inventory["lock_sha256"])
    return {"status": "COLLECTED_AND_VERIFIED", "packages": len(inventory["packages"]),
            "notice_files": len(payload) - 1, "inventory_sha256": sha(payload["DEPENDENCIES.json"]),
            "scope": inventory["scope"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--cargo-home", type=Path, default=os.environ.get("CARGO_HOME"), required=False)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.cargo_home is None:
        parser.error("explicit Cargo home is required")
    print(json.dumps(collect(json.loads(args.metadata.read_bytes()), args.lock.read_bytes(),
                             args.cargo_home.resolve(), args.output.resolve()), indent=2))


if __name__ == "__main__":
    main()
