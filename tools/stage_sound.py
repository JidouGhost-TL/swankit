"""Enroll sound oracle results without changing existing frozen expectations."""
import hashlib
import json
from pathlib import Path


def stage_sound(manifest_path, output, spec_dir="ws-sound"):
    repository = Path(__file__).resolve().parents[1]
    if spec_dir not in ("ws-sound", "ws-sound-extra", "ws-sound-captures"):
        raise ValueError("unknown sound spec selection")
    destination = output.resolve() / spec_dir
    if destination == repository or repository in destination.parents:
        raise ValueError("sound corpus must be outside the repository")
    if destination.exists():
        raise ValueError("sound spec directory already exists; frozen specs cannot be overwritten")
    manifest_path = manifest_path.resolve(strict=True)
    manifest = json.loads(manifest_path.read_text())
    records = []
    samples = {}
    provenance = {}
    for row in manifest["samples"]:
        name = row["name"]
        if not isinstance(name, str) or Path(name).name != name or name in (".", ".."):
            raise ValueError("sound sample name must be one path component")
        if name.startswith(".") or name in samples or Path(name).suffix.lower() in (".json", ".md", ".tsv", ".txt"):
            raise ValueError("duplicate or non-sample sound name")
        source = Path(row["source"]).resolve(strict=True)
        source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        if row["expect"].get("sha256") != source_hash:
            raise ValueError(f"{name}: source differs from independently frozen expectation")
        references = row["args"].get("references", [])
        if not references:
            raise ValueError(f"{name}: independent reference provenance required")
        for reference in references:
            path = Path(reference["path"]).resolve(strict=True)
            if hashlib.sha256(path.read_bytes()).hexdigest() != reference["sha256"]:
                raise ValueError(f"{name}: reference hash changed")
        samples[name] = {"args": row["args"], "expect": row["expect"]}
        provenance[name] = {"source": str(source), "sha256": source_hash,
                            "references": references}
        records.append((name, source))
    if not records:
        raise ValueError("sound manifest selects no samples")
    # Validate the whole selection before touching the new output directory.
    destination.mkdir(parents=True)
    for name, source in records:
        (destination / name).symlink_to(source)
    (destination / "spec.json").write_text(json.dumps(
        {"format": "ws-sound", "samples": samples,
         "excluded": {"README.md": "corpus documentation"}}, indent=2, sort_keys=True) + "\n")
    (destination / "versions.json").write_text(json.dumps({
        "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
        "samples": provenance,
    }, indent=2, sort_keys=True) + "\n")
    (destination / "README.md").write_text(
        "# Frozen sound corpus\n\n"
        f"{len(records)} samples; sources are read-only symlinks. "
        "Expectations are supplied by an independent oracle manifest. "
        "Provenance records source and reference hashes. Existing corpus specs "
        "and root provenance are never modified. Select a new directory for "
        "any further enrollment; never regenerate this spec.\n")
    print(f"Staged {len(records)} sound samples, 1 spec, 1 provenance ledger")
    return len(records)
