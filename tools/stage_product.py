"""Enroll product detection from existing independent, frozen owner specs."""
import fcntl
import hashlib
import json
from pathlib import Path


def stage_product(output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve(strict=True)
    if output == repository or repository in output.parents:
        raise ValueError("product corpus must be outside the repository")
    records = []
    for selection, identity in (("ws-rom", "ws-rom"), ("ws-patch", "ws-patch-ips")):
        folder = output / selection
        spec_bytes = (folder / "spec.json").read_bytes()
        spec = json.loads(spec_bytes)
        if spec["format"] != selection or not spec["samples"]:
            raise ValueError("invalid owner spec for product detection")
        for name, sample in sorted(spec["samples"].items()):
            if Path(name).name != name or name.startswith("."):
                raise ValueError("owner sample must be a single filename")
            source = (folder / name).resolve(strict=True)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            if digest != sample["expect"]["sha256"]:
                raise ValueError("owner sample differs from frozen hash")
            record = {
                "args": {"kind": selection, "reference": sample["expect"]},
                "expect": {"selected": identity, "sha256": digest},
            }
            records.append((source, record, selection, name,
                            hashlib.sha256(spec_bytes).hexdigest()))
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / "swankit-detection"
        if destination.exists():
            raise ValueError("product spec exists; frozen expectations cannot be overwritten")
        destination.mkdir()
        samples = {}
        provenance = {}
        for index, (source, record, selection, name, spec_hash) in enumerate(records):
            key = f"sample-{index:06d}.dat"
            (destination / key).symlink_to(source)
            samples[key] = record
            provenance[key] = {"owner_selection": selection, "owner_sample": name,
                               "owner_spec_sha256": spec_hash}
        (destination / "spec.json").write_text(json.dumps({
            "format": "swankit-detection", "samples": samples,
            "excluded": {"enrollment.json": "independent owner spec provenance"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps(provenance, indent=2,
                                                               sort_keys=True) + "\n")
    return len(records)
