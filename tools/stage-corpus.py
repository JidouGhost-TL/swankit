#!/usr/bin/env python3
"""Enroll existing title ROMs and independent facts in a fresh external spec root."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import subprocess


FIELDS = (
    "publisher_id", "publisher", "color", "game_id", "version", "rom_size_code",
    "rom_size_decl", "save_code", "save", "flags", "orientation", "bus_width",
    "rom_speed", "mapper", "checksum_footer", "checksum_computed", "checksum_ok",
    "reset_seg_off", "reset_linear", "footer_jmp_ok", "size_bytes", "size_pow2",
    "maintenance", "mapper_code", "footer_hex",
)
SPEED_SOURCE = "https://ws.nesdev.org/w/index.php?title=Memory_map&oldid=513"
FLAGS_SOURCE = "https://ws.nesdev.org/w/index.php?title=ROM_header&oldid=680"


def log(path, phase, done, next_step):
    if path:
        stamp = subprocess.check_output(["date", "-Iminutes"], text=True).strip()
        with Path(path).open("a") as out:
            out.write(f"{stamp} | {phase} | {done} | {next_step}\n")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def stage(args):
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    titles = args.titles.resolve(strict=True)
    if not titles.is_dir():
        raise ValueError("titles must be a directory")
    if output == root or root in output.parents:
        raise ValueError("corpus staging must be outside the repository")
    if output.exists():
        raise ValueError("staging already exists; frozen specs must never be overwritten")
    records = []
    corrections = []
    versions = {}
    for title in sorted(path for path in titles.iterdir() if path.is_dir()):
        rom_dir = title / "rom"
        roms = sorted(path for path in rom_dir.iterdir() if path.suffix.lower() in (".ws", ".wsc")) if rom_dir.is_dir() else []
        reference_path = title / "rom-facts.json"
        if not roms:
            if reference_path.exists():
                raise ValueError(f"{title.name}: facts without a ROM")
            continue
        if len(roms) != 1:
            raise ValueError(f"{title.name}: expected one ROM per title, got {len(roms)}")
        reference_bytes = reference_path.read_bytes()
        reference = json.loads(reference_bytes)
        rom = roms[0].resolve(strict=True)
        data = rom.read_bytes()
        if reference["file"] != roms[0].name or reference["size_bytes"] != len(data):
            raise ValueError(f"{title.name}: facts describe a different ROM")
        for name in ("md5", "sha1"):
            if hashlib.new(name, data).hexdigest() != reference[name]:
                raise ValueError(f"{title.name}: source {name} differs from frozen facts")
        raw = {field: reference[field] for field in FIELDS}
        expect = dict(raw)
        fixes = {}
        # The reference used the width bit as the speed bit. Hardware specifies
        # a one-cycle access plus the wait state from bit 3, independently of width.
        cycles = 1 + bool(int(raw["flags"], 16) & 8)
        speed = "1 cycle" if cycles == 1 else "2 cycles"
        if raw["rom_speed"] != speed:
            fix = {
                "oracle": raw["rom_speed"], "corrected": speed,
                "reason": "ROM wait state is flag bit 3; base byte access is one cycle",
                "sources": [FLAGS_SOURCE, SPEED_SOURCE],
            }
            fixes["rom_speed"] = fix
            expect["rom_speed"] = speed
            corrections.append({"title": title.name, "field": "rom_speed", **fix})
        digest = hashlib.sha256(data).hexdigest()
        expect["sha256"] = digest
        if digest in versions:
            raise ValueError(f"{title.name}: duplicate ROM SHA-256 requires explicit enrollment policy")
        versions[digest] = {
            "format": "ws-rom", "size": len(data),
            "reference_sha256": hashlib.sha256(reference_bytes).hexdigest(),
            "title": title.name,
        }
        name = title.name + rom.suffix.lower()
        records.append((name, rom, {"args": {"reference_facts": raw, "corrections": fixes}, "expect": expect}))
    if not records:
        raise ValueError("no title ROMs selected")
    # Discover and validate everything before creating a new enrollment root.
    sample_dir = output / "ws-rom"
    sample_dir.mkdir(parents=True)
    samples = {}
    for name, rom, sample in records:
        (sample_dir / name).symlink_to(rom)
        samples[name] = sample
    write_json(sample_dir / "spec.json", {"format": "ws-rom", "samples": samples})
    write_json(output / "versions.json", versions)
    write_json(output / "disagreements.json", corrections)
    (output / "README.md").write_text(
        "# Frozen swankit cartridge corpus staging\n\n"
        f"Enrolled {len(records)} ROMs from `{titles}`. ROMs are symlinks; no bytes copied.\n"
        f"Facts copied as JSON values from each title's rom-facts.json; {len(corrections)} documented corrections.\n"
        "The spec carries original facts in args.reference_facts and reviewed corrections in args.corrections.\n"
        "expect contains corrected facts plus independently computed SHA-256. versions.json records\n"
        "source-facts SHA-256, and disagreements.json lists every correction.\n\n"
        "To rebuild, select a NEW empty output path; never regenerate this frozen root:\n\n"
        f"```sh\npython3 {root / 'tools/stage-corpus.py'} --titles '{titles}' --output NEW_STAGING_ROOT\n"
        f"FORMATKIT_CORPUS_DIR='{output}' {root / 'tools/ci.sh'} --corpus\n```\n"
    )
    print(f"Staged {len(records)} ROM symlinks, 1 spec, {len(versions)} golden SHA-256 values, {len(corrections)} corrections")
    log(args.progress_log, "corpus staging", f"{len(records)} ROMs, {len(versions)} golden digests, {len(corrections)} corrections", "run safe corpus gate")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--titles", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--progress-log", default=os.environ.get("SWANKIT_PROGRESS_LOG"))
    args = parser.parse_args()
    log(args.progress_log, "corpus staging start", "0 ROMs enrolled", "validate existing ROMs and facts")
    try:
        stage(args)
    except (OSError, ValueError, KeyError) as error:
        log(args.progress_log, "corpus staging blocked", "0 specs accepted", str(error))
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
