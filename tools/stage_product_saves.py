"""Enroll composed save detection from frozen independent natural-save specs."""
import fcntl
import hashlib
import json
from pathlib import Path


def _positive(value, exclude):
    if isinstance(value, dict):
        return sum((1 if v is True and k in ("valid", "global_valid", "signature_valid",
                    "checksum_ok", "checksum_valid", "crc_ok", "ok") else 0)
                   + _positive(v, exclude) for k, v in value.items() if k not in exclude)
    if isinstance(value, list):
        return sum(_positive(v, exclude) for v in value)
    return 0


def stage_product_saves(output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve(strict=True)
    if output == repository or repository in output.parents:
        raise ValueError("product save corpus must be outside the repository")
    definitions = {}
    for path in (repository / "crates/ws-saves/data").glob("*.json"):
        value = json.loads(path.read_bytes())
        if (value.get("schema") != "swankit-save-def/1"
                or value["validity"].get("policy") != "program"
                or not isinstance(value["validity"].get("exclude"), list)
                or any(not isinstance(n, str) or not n for n in value["validity"]["exclude"])):
            raise ValueError("unsupported save definition validity policy")
        definitions[value["id"]] = value
    if not definitions:
        raise ValueError("save definitions are not integrated")
    records = []
    for namespace in ("ws-saves", "ws-saves-parity"):
        folder = output / namespace
        spec_bytes = (folder / "spec.json").read_bytes()
        spec = json.loads(spec_bytes)
        if spec["format"] != "ws-saves":
            raise ValueError("invalid save owner spec")
        enrollment = json.loads((output / (namespace + "-enrollment.json")).read_bytes())
        origins = {(r["id"], r["sha256"], r["name"]): r["path"]
                   for r in enrollment["records"]}
        for key, sample in sorted(spec["samples"].items()):
            if Path(key).name != key or key.startswith("."):
                raise ValueError("owner sample must be a single filename")
            source = (folder / key).resolve(strict=True)
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            if digest != sample["expect"]["sha256"]:
                raise ValueError("save differs from frozen source hash")
            identity = sample["args"]["id"]
            definition = definitions[identity]
            name = origins[(identity, digest, sample["args"]["name"])]
            context = {"schema": "swankit-save-context/1",
                       "tables": sample["args"].get("context", {}), "images": {}}
            companion = Path(name).with_suffix(".ieep")
            if companion.is_file():
                context["images"][str(companion)] = list(companion.read_bytes())
            decoded = sample["expect"]["decoded"]
            positive = _positive(decoded, definition["validity"]["exclude"]) > 0
            if isinstance(decoded, dict) and (decoded.get("error") is not None
                    or any(decoded.get(k) is False for k in ("valid", "global_valid"))):
                positive = False
            record = {"args": {"id": identity, "name": name, "context": context},
                      "expect": {"selected": "ws-save-image", "matched": positive,
                                 "sha256": digest, "decoded": decoded}}
            if "corrupt_offset" in sample["args"]:
                record["args"]["corrupt_offset"] = sample["args"]["corrupt_offset"]
                for field in ("repaired", "repaired_sha256"):
                    record["expect"][field] = sample["expect"][field]
            records.append((source, record, namespace, key,
                            hashlib.sha256(spec_bytes).hexdigest()))
    if not records:
        raise ValueError("no natural saves selected")
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / "swankit-save-detection"
        if destination.exists():
            raise ValueError("product save spec exists; frozen expectations cannot be overwritten")
        destination.mkdir()
        samples = {}
        provenance = {}
        for index, (source, record, namespace, key, spec_hash) in enumerate(records):
            name = f"sample-{index:06d}.dat"
            (destination / name).symlink_to(source)
            samples[name] = record
            provenance[name] = {"owner_selection": namespace, "owner_sample": key,
                                "owner_spec_sha256": spec_hash}
        (destination / "spec.json").write_text(json.dumps({
            "format": "swankit-save-detection", "samples": samples,
            "excluded": {"enrollment.json": "independent owner provenance"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps(provenance, indent=2,
                                                               sort_keys=True) + "\n")
    return len(records)
