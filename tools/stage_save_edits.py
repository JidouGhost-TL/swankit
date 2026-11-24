"""Enroll retained save inputs with independently recorded CLI edit outcomes, once."""
import fcntl
import hashlib
import json
from pathlib import Path

OPTIONS = ("--all-copies", "--no-repair", "--skip-validation")
REASONS = ("input-unusable", "post-edit-unusable")
MAX_SAVE_BYTES = 16 * 1024 * 1024


def _text(record, key):
    value = record.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"save edit record requires a nonempty {key}")
    return value


def _digest(value):
    if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise ValueError("invalid SHA-256 digest")
    return value


def _expect(record):
    expect = record.get("expect")
    if not isinstance(expect, dict):
        raise ValueError("save edit record requires an independent expectation")
    outcome = expect.get("outcome")
    if outcome == "written" and set(expect) == {"outcome", "output_sha256"}:
        return {"outcome": outcome, "output_sha256": _digest(expect["output_sha256"])}
    if outcome == "refused" and set(expect) == {"outcome", "reason"} and expect["reason"] in REASONS:
        return {"outcome": outcome, "reason": expect["reason"]}
    raise ValueError("save edit expectation must be written with output_sha256 or refused with a known reason")


SELECTION = "swankit-save-edits"


def stage_save_edits(manifest_path, output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve(strict=True)
    if output == repository or repository in output.parents:
        raise ValueError("save edit corpus must be outside the repository")
    manifest_bytes = manifest_path.read_bytes()
    records = json.loads(manifest_bytes)
    if not isinstance(records, list) or not records:
        raise ValueError("save edit manifest must contain independently expected records")
    selected = []
    for record in records:
        if not isinstance(record, dict):
            raise ValueError("invalid save edit record")
        source = Path(_text(record, "input_path")).resolve(strict=True)
        if not source.is_file() or not 0 < source.stat().st_size <= MAX_SAVE_BYTES:
            raise ValueError("save input is empty or exceeds bound")
        if hashlib.sha256(source.read_bytes()).hexdigest() != _digest(record.get("input_sha256")):
            raise ValueError("save input bytes differ from independent manifest")
        field = record.get("field")
        if type(field) is not int or not 0 <= field < 1 << 32:
            raise ValueError("invalid save field index")
        value = _text(record, "value")
        json.loads(value)
        options = record.get("options", [])
        if not isinstance(options, list) or any(o not in OPTIONS for o in options) or len(set(options)) != len(options):
            raise ValueError("unknown or repeated save edit option")
        name = _text(record, "name")
        if "/" in name or "\\" in name or name in (".", ".."):
            raise ValueError("save logical name must be a bare file name")
        _text(record, "oracle")
        selected.append((source, {
            "args": {"definition": _text(record, "definition"), "name": name, "field": field,
                     "value": value, "options": options},
            "expect": {"input_sha256": record["input_sha256"], **_expect(record)},
        }))
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / SELECTION
        if destination.exists():
            raise ValueError("save edit selection exists; frozen expectations cannot be overwritten")
        destination.mkdir()
        samples = {}
        for index, (source, sample) in enumerate(selected):
            name = f"sample-{index:06d}.dat"
            (destination / name).symlink_to(source)
            samples[name] = sample
        (destination / "spec.json").write_text(json.dumps({
            "format": "swankit-save-edits", "samples": samples,
            "excluded": {"enrollment.json": "independent edit outcome provenance"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps({
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(), "records": records,
        }, indent=2, sort_keys=True) + "\n")
    return len(selected)
