"""Enroll independent IPS/base/target evidence into a new frozen selection."""
import fcntl
import hashlib
import json
from pathlib import Path


def _hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stage_patch(manifest_path, output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve()
    if output == repository or repository in output.parents:
        raise ValueError("patch corpus must be outside the repository")
    manifest_path = manifest_path.resolve(strict=True)
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    records = []
    names = set()
    for row in manifest["samples"]:
        name = row["name"]
        if (not isinstance(name, str) or Path(name).name != name or name.startswith(".")
                or Path(name).suffix != ".ips" or name in names):
            raise ValueError("patch names must be unique IPS filenames")
        names.add(name)
        source = Path(row["source"]).resolve(strict=True)
        base = Path(row["base"]).resolve(strict=True)
        target = Path(row["target"]).resolve(strict=True)
        expect = row["expect"]
        for path, key, size_key in ((source, "sha256", None),
                                    (base, "base_sha256", "base_size"),
                                    (target, "patched_sha256", "patched_size")):
            if _hash(path) != expect[key]:
                raise ValueError(f"{name}: {key} differs from independent expectation")
            if size_key is not None and path.stat().st_size != expect[size_key]:
                raise ValueError(f"{name}: {size_key} differs from independent expectation")
        if not isinstance(expect["records"], int) or expect["records"] < 0:
            raise ValueError("invalid independent IPS record count")
        provenance = row["provenance"]
        references = provenance["references"]
        if not references or provenance["reference_kind"] not in (
                "published-digest", "archived-run-digest", "independent-python"):
            raise ValueError("independent patch reference provenance required")
        for reference in references:
            if _hash(Path(reference["path"]).resolve(strict=True)) != reference["sha256"]:
                raise ValueError(f"{name}: independent reference hash changed")
        records.append((name, source, base, target, expect, provenance))
    if not records:
        raise ValueError("patch manifest selects no samples")
    # Inspect the full selection before writing; serialize shared-root additions.
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / "ws-patch"
        if destination.exists():
            raise ValueError("patch spec exists; frozen specs must never be overwritten")
        destination.mkdir()
        (destination / "bases").mkdir()
        (destination / "targets").mkdir()
        samples = {}
        ledger = {}
        for name, source, base, target, expect, provenance in records:
            base_name = "bases/" + expect["base_sha256"] + ".rom"
            target_name = "targets/" + Path(name).stem + ".rom"
            if not (destination / base_name).exists():
                (destination / base_name).symlink_to(base)
            (destination / target_name).symlink_to(target)
            (destination / name).symlink_to(source)
            samples[name] = {"args": {"base": base_name, "target": target_name,
                                      "reference_kind": provenance["reference_kind"]},
                             "expect": expect}
            ledger[name] = {"source": str(source), "base": str(base), "target": str(target),
                            "expect": expect, "provenance": provenance}
        (destination / "spec.json").write_text(json.dumps({
            "format": "ws-patch", "samples": samples,
            "excluded": {"enrollment.json": "independent provenance ledger"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps({
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
            "samples": ledger,
        }, indent=2, sort_keys=True) + "\n")
    print(f"Staged {len(records)} IPS patches, independent targets and 1 frozen spec")
    return len(records)
