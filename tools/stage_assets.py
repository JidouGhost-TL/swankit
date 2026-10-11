"""Enroll per-ROM asset maps with independent decode evidence into a new frozen selection."""
import fcntl
import hashlib
import json
import re
from pathlib import Path


def _hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stage_assets(manifest_path, output, selection="ws-assets"):
    """Write the NEW selection `ws-assets` or `ws-assets-<suffix>` (later map revisions)."""
    if not re.fullmatch(r"ws-assets(-[a-z0-9]+)?", selection):
        raise ValueError("asset selections are named ws-assets or ws-assets-<suffix>")
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve()
    if output == repository or repository in output.parents:
        raise ValueError("asset corpus must be outside the repository")
    manifest = json.loads(manifest_path.resolve(strict=True).read_bytes())
    records = []
    names = set()
    for row in manifest["samples"]:
        name = row["name"]
        if (not isinstance(name, str) or Path(name).name != name or name.startswith(".")
                or Path(name).suffix != ".rom" or name in names):
            raise ValueError("asset sample names must be unique .rom filenames")
        names.add(name)
        rom = Path(row["rom"]).resolve(strict=True)
        amap = Path(row["map"]).resolve(strict=True)
        document = json.loads(amap.read_bytes())
        if document.get("schema") != "swankit-asset-map/1":
            raise ValueError(f"{name}: not a swankit-asset-map/1 document")
        if document["rom"]["sha256"] != _hash(rom):
            raise ValueError(f"{name}: map describes a different ROM")
        if any("file" in charmap for charmap in document.get("charmaps", {}).values()):
            raise ValueError(f"{name}: staged maps must use builtin or inline charmaps")
        # Independent evidence: decoded payload files produced by another tool.
        independent = {}
        ids = {a["id"] for a in document["assets"]}
        for asset_id, evidence in sorted(row.get("independent", {}).items()):
            if asset_id not in ids:
                raise ValueError(f"{name}: independent evidence for unknown asset {asset_id}")
            path = Path(evidence["path"]).resolve(strict=True)
            if _hash(path) != evidence["sha256"]:
                raise ValueError(f"{name}: independent evidence for {asset_id} changed")
            independent[asset_id] = evidence["sha256"]
        provenance = row["provenance"]
        if not provenance.get("independent_tool") or not provenance.get("map_source"):
            raise ValueError(f"{name}: provenance must name the map source and independent tool")
        records.append((name, rom, amap, independent, provenance))
    if not records:
        raise ValueError("asset manifest selects no samples")
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        destination = output / selection
        if destination.exists():
            raise ValueError("asset spec exists; frozen specs must never be overwritten")
        destination.mkdir()
        (destination / "maps").mkdir()
        samples = {}
        ledger = {}
        for name, rom, amap, independent, provenance in records:
            map_name = "maps/" + Path(name).stem + ".json"
            (destination / map_name).write_bytes(amap.read_bytes())
            (destination / name).symlink_to(rom)
            samples[name] = {"args": {"map": map_name,
                                      "independent_ids": {i: True for i in independent}},
                             "expect": {"rom_sha256": _hash(rom), "verify_errors": 0,
                                        "independent": independent}}
            ledger[name] = {"rom": str(rom), "map": str(amap), "map_sha256": _hash(amap),
                            "independent_count": len(independent), "provenance": provenance}
        (destination / "spec.json").write_text(json.dumps({
            "format": "ws-assets", "samples": samples,
            "excluded": {"enrollment.json": "independent provenance ledger",
                         "maps": "asset map documents read through sample args"},
        }, indent=2, sort_keys=True) + "\n")
        (destination / "enrollment.json").write_text(json.dumps(ledger, indent=2, sort_keys=True) + "\n")
    return len(records)
