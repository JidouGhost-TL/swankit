"""Enroll Python parity by copying frozen owner expectations without recomputing them."""
import fcntl
import hashlib
import json
from pathlib import Path


def stage_python(output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve(strict=True)
    if output == repository or repository in output.parents:
        raise ValueError("Python corpus must be outside the repository")
    records = []
    for selection, operation in (("swankit-detection", "detect"),
                                 ("swankit-save-detection", "save")):
        folder = output / selection
        raw_spec = (folder / "spec.json").read_bytes()
        spec = json.loads(raw_spec)
        if spec["format"] != selection or not spec["samples"]:
            raise ValueError("invalid frozen product spec")
        for name, sample in sorted(spec["samples"].items()):
            if Path(name).name != name or name.startswith("."):
                raise ValueError("owner sample must be a single filename")
            source = (folder / name).resolve(strict=True)
            if hashlib.sha256(source.read_bytes()).hexdigest() != sample["expect"]["sha256"]:
                raise ValueError("owner sample differs from frozen hash")
            args = dict(sample["args"], operation=operation)
            records.append((source, args, sample["expect"], selection, name,
                            hashlib.sha256(raw_spec).hexdigest()))
    codec_folder = output / "ws-compress"
    raw_spec = (codec_folder / "spec.json").read_bytes()
    codec_spec = json.loads(raw_spec)
    if codec_spec["format"] != "ws-compress" or not codec_spec["samples"]:
        raise ValueError("invalid frozen codec spec")
    enrollment = json.loads((output / "ws-compress-enrollment.json").read_bytes())
    origins = {(r["codec"], r["stored_sha256"], r["provenance"]): r
               for r in enrollment["records"]}
    for name, sample in sorted(codec_spec["samples"].items()):
        expect = sample["expect"]
        origin = origins[(expect["codec"], expect["stored_sha256"], sample["args"]["provenance"])]
        source = Path(origin["stored_path"]).resolve(strict=True)
        if hashlib.sha256(source.read_bytes()).hexdigest() != expect["stored_sha256"]:
            raise ValueError("codec source differs from frozen hash")
        args = dict(sample["args"], operation="codec", raw_len=expect["raw_len"])
        records.append((source, args, expect, "ws-compress", name,
                        hashlib.sha256(raw_spec).hexdigest()))
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / "swankit-python"
        if destination.exists():
            raise ValueError("Python spec exists; frozen expectations cannot be overwritten")
        destination.mkdir()
        samples, provenance = {}, {}
        for index, (source, args, expect, selection, owner_name, spec_hash) in enumerate(records):
            name = f"sample-{index:06d}.dat"
            (destination / name).symlink_to(source)
            samples[name] = {"args": args, "expect": expect}
            provenance[name] = {"owner_selection": selection, "owner_sample": owner_name,
                                "owner_spec_sha256": spec_hash}
        (destination / "spec.json").write_text(json.dumps({
            "format": "swankit-python", "samples": samples,
            "excluded": {"enrollment.json": "frozen owner provenance"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps(provenance, indent=2,
                                                               sort_keys=True) + "\n")
    return len(records)
