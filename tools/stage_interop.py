"""Enroll fresh Java interop vectors; never modify existing frozen selections."""
import fcntl
import hashlib
import json
from pathlib import Path


def stage_interop(manifest_path, output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve(strict=True)
    if output == repository or repository in output.parents:
        raise ValueError("interop corpus must be outside the repository")
    manifest_bytes = manifest_path.read_bytes()
    records = json.loads(manifest_bytes)
    if not isinstance(records, list) or not records:
        raise ValueError("interop manifest must contain independent Java records")
    selected = []
    for record in records:
        if not isinstance(record, dict) or not all(key in record for key in (
                "input_path", "input_sha256", "operation", "output_bytes", "output_sha256", "oracle")):
            raise ValueError("interop record lacks required oracle fields")
        if not isinstance(record["input_path"], str):
            raise ValueError("interop input path must be a string")
        path = Path(record["input_path"]).resolve(strict=True)
        if path.stat().st_size > 64 * 1024 * 1024:
            raise ValueError("interop input exceeds bound")
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != record["input_sha256"]:
            raise ValueError("interop input differs from oracle manifest")
        operation = record["operation"]
        size = record["output_bytes"]
        if type(operation) is not int or operation not in (1, 2, 3) or type(size) is not int or not 0 < size <= 262144:
            raise ValueError("invalid interop operation or output size")
        if (operation == 1 and size != 72) or (operation in (2, 3) and size % 256):
            raise ValueError("interop output size does not match the operation contract")
        expected = record["output_sha256"]
        if not isinstance(expected, str) or len(expected) != 64 or any(c not in "0123456789abcdef" for c in expected):
            raise ValueError("invalid independent output digest")
        if not isinstance(record["oracle"], str) or not record["oracle"].strip():
            raise ValueError("independent Java provenance required")
        selected.append((path, {
            "args": {"operation": operation},
            "expect": {"sha256": digest, "output_sha256": expected, "output_bytes": size},
        }))
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / "swankit-interop"
        if destination.exists():
            raise ValueError("interop selection exists; frozen expectations cannot be overwritten")
        destination.mkdir()
        samples = {}
        for index, (path, sample) in enumerate(selected):
            name = f"sample-{index:06d}.dat"
            (destination / name).symlink_to(path)
            samples[name] = sample
        (destination / "spec.json").write_text(json.dumps({
            "format": "swankit-interop", "samples": samples,
            "excluded": {"enrollment.json": "independent Java oracle provenance"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps({
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(), "records": records,
        }, indent=2, sort_keys=True) + "\n")
    return len(selected)
