"""Enroll recorded relay payloads against independent byte oracles, once."""
import fcntl
import hashlib
import json
from pathlib import Path


def stage_link(manifest_path, output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve(strict=True)
    if output == repository or repository in output.parents:
        raise ValueError("link corpus must be outside the repository")
    manifest_bytes = manifest_path.read_bytes()
    records = json.loads(manifest_bytes)
    if not isinstance(records, list) or not records:
        raise ValueError("link manifest must contain independently expected records")
    selected = []
    for record in records:
        if not isinstance(record, dict):
            raise ValueError("invalid link record")
        source = Path(record["input_path"]).resolve(strict=True)
        reference = Path(record["reference_path"]).resolve(strict=True)
        for path in (source, reference):
            if not path.is_file() or not 0 < path.stat().st_size <= 1024 * 1024:
                raise ValueError("link payload is empty or exceeds bound")
        source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        reference_bytes = reference.read_bytes()
        reference_hash = hashlib.sha256(reference_bytes).hexdigest()
        if source_hash != record["input_sha256"] or reference_hash != record["output_sha256"]:
            raise ValueError("link bytes differ from independent manifest")
        if type(record["output_bytes"]) is not int or record["output_bytes"] != len(reference_bytes):
            raise ValueError("link output size differs from reference")
        if not isinstance(record["oracle"], str) or not record["oracle"].strip():
            raise ValueError("independent link oracle provenance required")
        direction = record["direction"]
        if direction not in ("A>B", "B>A"):
            raise ValueError("invalid link direction")
        options = {key: record[key] for key in ("latency_ms", "jitter_ms", "seed")}
        for key, value in options.items():
            limit = (1 << 64) - 1 if key == "seed" else 600000
            if type(value) is not int or not 0 <= value <= limit:
                raise ValueError("invalid bounded link schedule option")
        forwarded = [0, 0]
        forwarded[direction == "B>A"] = len(reference_bytes)
        selected.append((source, {
            "args": {"direction": direction, **options},
            "expect": {"sha256": source_hash, "output_sha256": reference_hash,
                       "output_bytes": len(reference_bytes), "forwarded": forwarded,
                       "dropped": [0, 0]},
        }))
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / "ws-link"
        if destination.exists():
            raise ValueError("link selection exists; frozen expectations cannot be overwritten")
        destination.mkdir()
        samples = {}
        for index, (source, sample) in enumerate(selected):
            name = f"sample-{index:06d}.dat"
            (destination / name).symlink_to(source)
            samples[name] = sample
        (destination / "spec.json").write_text(json.dumps({
            "format": "ws-link", "samples": samples,
            "excluded": {"enrollment.json": "independent byte oracle provenance"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps({
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(), "records": records,
        }, indent=2, sort_keys=True) + "\n")
    return len(selected)
