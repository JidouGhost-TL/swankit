#!/usr/bin/env python3
"""Enroll existing title ROMs and independent facts in a fresh external spec root."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import subprocess
import struct
import fcntl


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


def stage_codecs(args):
    """Add a new frozen codec namespace from an independent oracle manifest.

    The manifest and its binary inputs are external. Never derive expectations
    from the Rust implementation under test, or change an existing namespace.
    """
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if output == root or root in output.parents:
        raise ValueError("corpus staging must be outside the repository")
    manifest_path = args.codec_manifest.resolve(strict=True)
    manifest = json.loads(manifest_path.read_bytes())
    records = manifest["records"]
    if not records:
        raise ValueError("codec manifest has no records")
    sample_dir = output / "ws-compress"
    if sample_dir.exists():
        raise ValueError("codec namespace already exists; frozen specs must never be overwritten")
    if (output / "ws-compress-enrollment.json").exists():
        raise ValueError("codec enrollment already exists")
    staged = []
    codecs = set()
    for record in records:
        codec = record["codec"]
        if codec not in ("lz-rle-7f", "lz-bits-16"):
            raise ValueError("unknown codec in manifest")
        codecs.add(codec)
        raw = Path(record["raw_path"]).read_bytes()
        stored = Path(record["stored_path"]).read_bytes()
        encoded = Path(record["encoded_path"]).read_bytes()
        for kind, data in (("raw", raw), ("stored", stored), ("encoded", encoded)):
            if hashlib.sha256(data).hexdigest() != record[kind + "_sha256"]:
                raise ValueError(f"independent {kind} digest differs from oracle manifest")
        if len(raw) > 65536 or record["consumed"] != len(stored):
            raise ValueError("invalid codec oracle extent")
        if record["require_fit"] and len(encoded) > len(stored):
            raise ValueError("reference encoder exceeds a required original slot")
        label = codec.encode()
        provenance = record["provenance"].encode()
        pair = (b"SCEPAIR1" + struct.pack("<IIQQ", len(label), len(provenance), len(raw), len(stored))
                + label + provenance + raw + stored)
        key = hashlib.sha256(label + b"\0" + raw + b"\0" + stored).hexdigest()[:32] + ".pair"
        expect = {
            "codec": codec, "raw_len": len(raw), "stored_len": len(stored),
            "raw_sha256": record["raw_sha256"], "stored_sha256": record["stored_sha256"],
            "consumed": record["consumed"], "encoded_len": len(encoded),
            "encoded_sha256": record["encoded_sha256"], "fits_original": len(encoded) <= len(stored),
        }
        sample = {"args": {"codec": codec, "require_fit": record["require_fit"],
                           "provenance": record["provenance"], "evidence": record["evidence"]}, "expect": expect}
        staged.append((codec, key, pair, sample))
    for codec in codecs:
        if (output / "_compress_pairs" / codec).exists():
            raise ValueError("codec pairs already exist; frozen pairs must never be overwritten")
    # Discovery and independent digest checks finish before any write.
    sample_dir.mkdir(parents=True)
    samples = {}
    unique = set()
    for index, (codec, key, pair, sample) in enumerate(staged):
        pair_path = output / "_compress_pairs" / codec / key
        if (codec, key) not in unique:
            pair_path.parent.mkdir(parents=True, exist_ok=True)
            pair_path.write_bytes(pair)
            unique.add((codec, key))
        name = f"sample-{index:06d}.pair"
        (sample_dir / name).symlink_to(pair_path)
        samples[name] = sample
    write_json(sample_dir / "spec.json", {"format": "ws-compress", "samples": samples})
    write_json(output / "ws-compress-enrollment.json", {
        "oracle_manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
        "records": records, "oracle_sources": manifest["oracle_sources"],
        "records_count": len(records), "unique_pairs": len(unique),
    })
    print(f"Staged {len(staged)} codec records, {len(unique)} unique Formatkit pairs, 1 frozen spec")
    log(args.progress_log, "codec staging", f"{len(staged)} codec records, {len(unique)} unique pairs", "run safe corpus gate")


def stage_graphics(args):
    """Add one new graphics directory; preserve every existing frozen expectation."""
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if output == root or root in output.parents:
        raise ValueError("corpus staging must be outside the repository")
    manifest_bytes = args.gfx_manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    format_name = getattr(args, "gfx_format", "ws-gfx")
    records = []
    for record in manifest["samples"]:
        name = record["name"]
        if (Path(name).name != name or name.startswith(".")
                or Path(name).suffix not in (".tiles", ".snapshot", ".display")):
            raise ValueError("invalid graphics sample name")
        source = Path(record["path"]).resolve(strict=True)
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        if record["expect"]["sha256"] != digest:
            raise ValueError(f"{name}: source differs from independent expectation")
        reference = record["reference_sha256"]
        if len(reference) != 64 or any(c not in "0123456789abcdef" for c in reference):
            raise ValueError(f"{name}: invalid reference provenance")
        records.append((name, source, digest, record))
    if not records or len({r[0] for r in records}) != len(records):
        raise ValueError("graphics enrollment must contain unique samples")
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        sample_dir = output / format_name
        if sample_dir.exists():
            raise ValueError("graphics spec exists; frozen expectations must never be overwritten")
        versions_path = output / "versions.json"
        versions = json.loads(versions_path.read_text()) if versions_path.exists() else {}
        additions = {}
        for name, source, digest, record in records:
            if digest in versions or digest in additions:
                raise ValueError(f"{name}: duplicate input digest requires explicit enrollment policy")
            additions[digest] = {"format": format_name, "size": source.stat().st_size,
                                 "reference_sha256": record["reference_sha256"],
                                 "provenance": record.get("provenance", {})}
        sample_dir.mkdir()
        samples = {}
        for name, source, _, record in records:
            (sample_dir / name).symlink_to(source)
            samples[name] = {"args": record["args"], "expect": record["expect"]}
        write_json(sample_dir / "spec.json", {
            "format": format_name, "samples": samples,
            "excluded": {"enrollment.json": "provenance sidecar, not a graphics byte stream"}})
        versions.update(additions)
        write_json(versions_path, versions)
        write_json(sample_dir / "enrollment.json", {
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
            "samples": len(records), "provenance": manifest.get("provenance", {})})
    print(f"Staged {len(records)} graphics symlinks and independent golden expectations")
    log(args.progress_log, "graphics staging", f"{len(records)} samples, existing golden values preserved",
        "run serialized required corpus gate")


def stage_text(args):
    """Enroll independent text/font expectations once in their own namespace.

    Text and cartridge handlers may share a ROM digest. Text provenance stays
    in a separate enrollment ledger; existing versioned expectations are not
    changed to assign the same input to a second format.
    """
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if output == root or root in output.parents:
        raise ValueError("corpus staging must be outside the repository")
    spec_dir = getattr(args, "text_spec_dir", "ws-text")
    if spec_dir not in ("ws-text", "ws-text-extra"):
        raise ValueError("invalid text spec directory")
    manifest_bytes = args.text_manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    records = []
    strings = glyphs = 0
    for record in manifest["samples"]:
        name = record["name"]
        if Path(name).name != name or name.startswith(".") or Path(name).suffix != ".text":
            raise ValueError("invalid text sample name")
        if not isinstance(record.get("args"), dict) or not isinstance(record.get("expect"), dict):
            raise ValueError("text sample args and expect must be objects")
        string_records = record["args"].get("strings", [])
        font_records = record["args"].get("fonts", [])
        if not isinstance(string_records, list) or not isinstance(font_records, list):
            raise ValueError("text strings and fonts must be arrays")
        strings += len(string_records)
        for font in font_records:
            if not isinstance(font, dict) or type(font.get("count")) is not int or font["count"] < 0:
                raise ValueError("invalid font count in text manifest")
            glyphs += font["count"]
        source = Path(record["path"]).resolve(strict=True)
        if hashlib.sha256(source.read_bytes()).hexdigest() != record["expect"]["sha256"]:
            raise ValueError("text source differs from independent expectation")
        references = record["references"]
        if not isinstance(references, list) or not references:
            raise ValueError("text enrollment needs independent oracle provenance")
        for reference in references:
            if hashlib.sha256(Path(reference["path"]).read_bytes()).hexdigest() != reference["sha256"]:
                raise ValueError("text oracle source differs from enrollment provenance")
        records.append((name, source, record))
    if not records or len({r[0] for r in records}) != len(records):
        raise ValueError("text enrollment must contain unique samples")
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        directory = output / spec_dir
        if directory.exists():
            raise ValueError("text spec exists; frozen expectations must never be overwritten")
        directory.mkdir()
        samples = {}
        for name, source, record in records:
            (directory / name).symlink_to(source)
            samples[name] = {"args": record["args"], "expect": record["expect"]}
        write_json(directory / "spec.json", {
            "format": "ws-text", "samples": samples,
            "excluded": {"enrollment.json": "independent text/font oracle provenance sidecar"},
            "source_excluded": manifest.get("source_excluded", {})})
        write_json(directory / "enrollment.json", {
            "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
            "samples": len(records), "references": {r[0]: r[2]["references"] for r in records},
            "provenance": manifest.get("provenance", {})})
    print(f"Staged {len(records)} text sources, {strings} strings, {glyphs} glyphs; frozen values preserved")
    log(args.progress_log, "text staging", f"{len(records)} sources, {strings} strings, {glyphs} glyphs",
        "run serialized required text corpus gate")


def stage_saves(args):
    """Enroll independent save parser trees in a new immutable namespace."""
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    if output == root or root in output.parents:
        raise ValueError("corpus staging must be outside the repository")
    manifest_path = args.save_manifest.resolve(strict=True)
    manifest = json.loads(manifest_path.read_bytes())
    namespace = args.save_namespace
    if namespace not in ("ws-saves", "ws-saves-parity", "ws-saves-play", "ws-saves-natural"):
        raise ValueError("unknown save namespace")
    sample_dir = output / namespace
    enrollment = output / (namespace + "-enrollment.json")
    if sample_dir.exists() or enrollment.exists():
        raise ValueError("save namespace exists; frozen expectations cannot be overwritten")
    records = manifest["records"]
    if not records:
        raise ValueError("save manifest has no independent records")
    samples = {}
    selected = []
    for index, record in enumerate(records):
        path = Path(record["path"]).resolve(strict=True)
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != record["sha256"]:
            raise ValueError("save bytes differ from independent oracle manifest")
        name = f"sample-{index:06d}" + path.suffix
        args_value = {"id": record["id"], "name": record["name"],
                      "context": record.get("context", {})}
        expect = {"decoded": record["decoded"], "sha256": record["sha256"]}
        if "corrupt_offset" in record:
            offset = record["corrupt_offset"]
            if not isinstance(offset, int) or not 0 <= offset < len(raw):
                raise ValueError("invalid save corruption offset")
            args_value["corrupt_offset"] = offset
            expect.update(repaired=record["repaired"], repaired_sha256=record["repaired_sha256"])
        samples[name] = {"args": args_value, "expect": expect}
        selected.append((name, path))
    sample_dir.mkdir(parents=True)
    for name, path in selected:
        (sample_dir / name).symlink_to(path)
    write_json(sample_dir / "spec.json", {"format": "ws-saves", "samples": samples})
    write_json(enrollment, {"manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
                            "normalization": manifest["normalization"], "records": records,
                            "source_excluded": manifest.get("notes", [])})
    print(f"Staged {len(records)} save records, 1 frozen spec")
    log(args.progress_log, "save staging", f"{len(records)} independent save trees and digests", "run serialized parity")



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument("--titles", type=Path)
    selection.add_argument("--codec-manifest", type=Path)
    selection.add_argument("--wondergate-manifest", type=Path,
                           help="enroll independent adapter taps and TCP peers in a NEW frozen adapter selection")
    selection.add_argument("--wondergate-server-manifest", type=Path,
                           help="enroll shared vectors, title captures or ranking histories in a NEW frozen server selection")
    selection.add_argument("--interop-manifest", type=Path,
                           help="enroll independent Java header/tile vectors in a NEW swankit-interop selection")
    selection.add_argument("--link-manifest", type=Path,
                           help="enroll recorded relay payloads and independent byte oracles in a NEW ws-link selection")
    selection.add_argument("--save-manifest", type=Path)
    parser.add_argument("--save-namespace", default="ws-saves")
    selection.add_argument("--gfx-manifest", type=Path,
                           help="enroll independently rendered graphics snapshots/fixtures into a new spec directory")
    selection.add_argument("--text-manifest", type=Path,
                           help="enroll independently dumped text and font glyph expectations")
    parser.add_argument("--text-spec-dir", choices=("ws-text", "ws-text-extra"), default="ws-text",
                        help="new text selection; never updates an existing selection")
    parser.add_argument("--gfx-format", choices=("ws-gfx", "ws-gfx-display"), default="ws-gfx",
                        help="graphics directory to enroll, each may be created only once")
    selection.add_argument("--sound-manifest", type=Path,
                           help="enroll independent sound expectations in a NEW ws-sound spec directory")
    selection.add_argument("--python-bindings", action="store_true",
                           help="enroll Python parity from existing frozen product and codec selections")
    selection.add_argument("--product-saves", action="store_true",
                           help="enroll composed natural-save detection and repair from frozen owner specs")
    selection.add_argument("--product-detection", action="store_true",
                           help="enroll composed detection from existing frozen ROM and IPS selections")
    selection.add_argument("--assets-manifest", type=Path,
                           help="enroll per-ROM asset maps with independent decodes in a NEW ws-assets selection")
    selection.add_argument("--patch-manifest", type=Path,
                           help="enroll independent IPS/base/target evidence in a NEW ws-patch selection")
    parser.add_argument("--sound-spec-dir", choices=("ws-sound", "ws-sound-extra", "ws-sound-captures"), default="ws-sound",
                        help="new sound selection; never updates an existing selection")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--progress-log", default=os.environ.get("SWANKIT_PROGRESS_LOG"))
    args = parser.parse_args()
    log(args.progress_log, "corpus staging start", "0 samples enrolled", "validate independent inputs")
    try:
        if args.wondergate_manifest:
            from stage_wondergate_adapter import stage_wondergate_adapter
            count = stage_wondergate_adapter(args.wondergate_manifest, args.output)
            log(args.progress_log, "adapter corpus staging", f"{count} independent taps, 1 frozen spec", "run safe adapter replay gate")
        elif args.link_manifest:
            from stage_link import stage_link
            count = stage_link(args.link_manifest, args.output)
            log(args.progress_log, "link corpus staging", f"{count} byte vectors, 1 frozen spec", "run serialized required corpus gate")
        elif args.wondergate_server_manifest:
            from stage_wondergate_server import stage_wondergate_server
            count = stage_wondergate_server(args.wondergate_server_manifest, args.output)
            log(args.progress_log, "shared service staging", f"{count} independent vectors, 1 frozen spec", "run serialized required corpus gate")
        elif args.interop_manifest:
            from stage_interop import stage_interop
            count = stage_interop(args.interop_manifest, args.output)
            log(args.progress_log, "interop staging", f"{count} Java vectors, 1 new frozen spec", "run safe corpus gate")
        elif args.save_manifest:
            stage_saves(args)
        elif args.python_bindings:
            from stage_python import stage_python
            count = stage_python(args.output)
            log(args.progress_log, "Python corpus staging", f"{count} samples, 1 new frozen spec", "run installed wheel parity through safe runner")
        elif args.product_saves:
            from stage_product_saves import stage_product_saves
            count = stage_product_saves(args.output)
            log(args.progress_log, "product save staging", f"{count} samples, 1 new spec", "run save detection and CLI parity through safe runner")
        elif args.product_detection:
            from stage_product import stage_product
            count = stage_product(args.output)
            log(args.progress_log, "product detection staging", f"{count} samples, 1 new spec", "run composed detection through safe runner")

        elif args.codec_manifest:
            stage_codecs(args)
        elif args.gfx_manifest:
            stage_graphics(args)
        elif args.sound_manifest:
            from stage_sound import stage_sound
            count = stage_sound(args.sound_manifest, args.output, args.sound_spec_dir)
            log(args.progress_log, "sound corpus staging", f"{count} samples, 1 frozen spec; existing specs unchanged", "run safe corpus gate")
        elif args.text_manifest:
            stage_text(args)
        elif args.assets_manifest:
            from stage_assets import stage_assets
            count = stage_assets(args.assets_manifest, args.output)
            log(args.progress_log, "asset map staging", f"{count} maps, 1 frozen spec; existing specs unchanged", "run safe corpus gate")
        elif args.patch_manifest:
            from stage_patch import stage_patch
            count = stage_patch(args.patch_manifest, args.output)
            log(args.progress_log, "patch corpus staging", f"{count} patches, 1 frozen spec; existing specs unchanged", "run safe corpus gate")
        else:
            stage(args)
    except (OSError, ValueError, KeyError, TypeError) as error:
        log(args.progress_log, "corpus staging blocked", "0 specs accepted", str(error))
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
