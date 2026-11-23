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

# These published patch versions omit the workspace notices. Their paired
# core archives contain the upstream root notices at the same release commit.
PATCH_PACKAGE = "formatkit-patch"
PATCH_NOTICES = {
    "LICENSE-APACHE": "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    "LICENSE-MIT": "06048a247dcb30b7118914e5fab9cc8e5d45f603b3c746fafc57f8fa62614662",
}
PATCH_ARCHIVES = {
    "0.1.1": {
        "patch_checksum": "3f52f0aa583a4aa23467a59c9a90e95a4c6b01a86308acf5cabbaf35e539fb3b",
        "core_checksum": "44b87354d0873192aa4a6dd5d0376cbaa377efef7874c5fe3efe93618c41490d",
        "commit": "0b4d49adcbd28eebcc4ab48c4c5e4a1dd7ce213b",
        "notices": PATCH_NOTICES,
    },
    "0.1.4": {
        "patch_checksum": "bf645547df1217294d86c6f1095770efd4fa0daaa8e6a8d45c8025b05232edd7",
        "core_checksum": "bd99a028d234f1d7ee95e0c708eb011491661a1e628885151398b15c36876f20",
        "commit": "3ee29c9c8ab6e7c769bcadd14e7ebed753af79f2",
        "notices": {**PATCH_NOTICES, "NOTICE": "4fb3ac4cf8f901abf4065fab736992f17a61aff6e63107b8e7209e8f3588a0fe"},
    },
}


def patch_notice_source(version):
    pin = PATCH_ARCHIVES[version]
    return {"repository": "https://github.com/JidouGhost-TL/formatkit", "commit": pin["commit"],
            "crate": "formatkit-core", "version": version, "checksum": pin["core_checksum"]}


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
        if key[0] == PATCH_PACKAGE and key[1] in PATCH_ARCHIVES:
            pin = PATCH_ARCHIVES[key[1]]
            notices = {x["source_path"]: x["sha256"] for x in package["licenses"]}
            if (package.get("notice_source") != patch_notice_source(key[1])
                    or package.get("registry_checksum") != pin["patch_checksum"]
                    or notices != pin["notices"] or len(package["licenses"]) != len(pin["notices"])):
                raise ValueError("Formatkit patch notice provenance differs")
    actual = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError("symbolic dependency notice")
        if path.is_file():
            actual.add(path.relative_to(root).as_posix())
    if actual != expected:
        raise ValueError("dependency notice inventory coverage differs")
    return inventory


def registry_archive(cargo_home, name, version, checksum):
    archives = list((cargo_home / "registry/cache").glob("*/" + name + "-" + version + ".crate"))
    if len(archives) != 1:
        raise ValueError("missing or ambiguous published archive: " + name)
    raw = archives[0].read_bytes()
    if sha(raw) != checksum:
        raise ValueError("published archive checksum differs: " + name)
    prefix = name + "-" + version + "/"
    members = {}
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
        for member in archive:
            if member.isfile() and member.name.startswith(prefix):
                leaf = member.name[len(prefix):]
                relative(leaf)
                if leaf in members:
                    raise ValueError("duplicate published archive member")
                members[leaf] = archive.extractfile(member).read()
    return members


def patch_notices(cargo_home, patch_archive, patch_checksum, version):
    pin = PATCH_ARCHIVES[version]
    evidence = patch_notice_source(version)
    core_archive = registry_archive(cargo_home, evidence["crate"], evidence["version"], evidence["checksum"])
    pairs = ((patch_archive, PATCH_PACKAGE), (core_archive, evidence["crate"]))
    if patch_checksum != pin["patch_checksum"]:
        raise ValueError("Formatkit patch archive identity differs")
    for members, name in pairs:
        manifest = tomllib.loads(members["Cargo.toml"].decode())["package"]
        source = json.loads(members[".cargo_vcs_info.json"])["git"]
        if (manifest["name"] != name or manifest["version"] != evidence["version"]
                or manifest.get("repository") != evidence["repository"]
                or manifest.get("license") != "MIT OR Apache-2.0"
                or source["sha1"] != evidence["commit"] or source.get("dirty")):
            raise ValueError("Formatkit shared notice source does not match release commit")
    files = {name: core_archive[name] for name in pin["notices"]}
    if {name: sha(raw) for name, raw in files.items()} != pin["notices"]:
        raise ValueError("Formatkit notice bytes do not match published source")
    return files


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
            checksum = pin["checksum"]
            archive_members = registry_archive(cargo_home, name, version, checksum)
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
        record = {"name": name, "version": version, "declared_license": package.get("license"),
                  "source": source or "workspace", "registry_checksum": checksum, "licenses": []}
        files = {}
        if name == PATCH_PACKAGE and version in PATCH_ARCHIVES:
            if archive_members is None or selected:
                raise ValueError("unexpected Formatkit patch license input")
            files = patch_notices(cargo_home, archive_members, checksum, version)
            record["notice_source"] = patch_notice_source(version)
        else:
            for rel in selected:
                path = root / rel
                if path.is_symlink() or not path.is_file():
                    raise ValueError("missing or symbolic declared license: " + name)
                raw = path.read_bytes()
                if archive_members is not None and archive_members.get(rel) != raw:
                    raise ValueError("license differs from published archive: " + name)
                files[rel] = raw
        if not files or not (package.get("license") or license_file):
            raise ValueError("missing declared license evidence: " + name)
        for rel, raw in sorted(files.items()):
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
