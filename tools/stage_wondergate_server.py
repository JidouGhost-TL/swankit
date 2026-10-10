"""Enroll independent shared HTTP/BPL bytes once; keep every golden external."""
import fcntl
import hashlib
import json
from pathlib import Path


def sha(data):
    return hashlib.sha256(data).hexdigest()


def checked_bytes(path, expected, maximum):
    source = Path(path).resolve(strict=True)
    if source.stat().st_size > maximum:
        raise ValueError("service oracle file exceeds bound")
    data = source.read_bytes()
    if sha(data) != expected:
        raise ValueError("service bytes differ from independent oracle manifest")
    return source, data


def stage_wondergate_server(manifest_path, output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve()
    if output == repository or repository in output.parents:
        raise ValueError("service corpus must be outside the repository")
    manifest_bytes = manifest_path.read_bytes()
    if len(manifest_bytes) > 8 * 1024 * 1024:
        raise ValueError("oversized service oracle manifest")
    manifest = json.loads(manifest_bytes)
    if manifest.get("schema") != "swankit-wondergate-services-manifest/1":
        raise ValueError("unknown service oracle schema")
    records = manifest.get("samples")
    if not isinstance(records, list) or not records:
        raise ValueError("service oracle samples required")
    selected = []
    for record in records:
        protocol = record["protocol"]
        clock = record["clock"]
        seed = record["seed"]
        if protocol not in ("HTTP", "BPL") or not isinstance(clock, str) or len(clock) != 14 or not clock.isascii() or not clock.isdigit():
            raise ValueError("invalid service protocol or fixed clock")
        from datetime import datetime, timezone
        # The Rust clock also reserves enough calendar space for JST.
        stamp = datetime.strptime(clock, "%Y%m%d%H%M%S").replace(tzinfo=timezone.utc)
        if stamp.year == 9999 and (stamp.month, stamp.day, stamp.hour) >= (12, 31, 15):
            raise ValueError("service clock overflows JST")
        if type(seed) is not int or not 0 <= seed < 2**64 or type(record["error"]) is not bool:
            raise ValueError("invalid service seed or error expectation")
        if not isinstance(record.get("oracle"), str) or not record["oracle"].strip():
            raise ValueError("independent service provenance required")
        source, request = checked_bytes(record["input_path"], record["input_sha256"], 81925)
        _, response = checked_bytes(record["response_path"], record["response_sha256"], 81924)
        if type(record["response_length"]) is not int or len(response) != record["response_length"]:
            raise ValueError("service oracle response length mismatch")
        if protocol == "BPL" and len(response) != (0 if record["error"] else 24):
            raise ValueError("BPL oracle must use the PHASE0 24-byte reply")
        selected.append((source, {
            "args": {"protocol": protocol, "clock": clock, "seed": seed},
            "expect": {"sha256": sha(request), "response_sha256": sha(response),
                       "response_length": len(response), "error": record["error"]},
        }))
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        directory = output / "ws-wondergate-services"
        if directory.exists():
            raise ValueError("service selection exists; frozen expectations can never be overwritten")
        directory.mkdir()
        samples = {}
        for index, (source, sample) in enumerate(selected):
            name = f"sample-{index:06d}.request"
            (directory / name).symlink_to(source)
            samples[name] = sample
        (directory / "spec.json").write_text(json.dumps({
            "format": "ws-wondergate-services", "samples": samples,
            "excluded": {"enrollment.json": "independent oracle provenance sidecar"},
        }, indent=2, sort_keys=True) + "\n")
        (directory / "enrollment.json").write_text(json.dumps({
            "manifest_sha256": sha(manifest_bytes), "samples": records,
        }, indent=2, sort_keys=True) + "\n")
    return len(selected)
