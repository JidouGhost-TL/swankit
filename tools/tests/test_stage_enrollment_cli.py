"""Enrollment commands extend one frozen external root without replacing specs."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "stage-corpus.py"


class EnrollmentCliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.output = root / "staging"
        cartridge = self.output / "ws-rom" / "spec.json"
        cartridge.parent.mkdir(parents=True)
        cartridge.write_text("frozen cartridge spec")
        self.versions = {"existing": {"format": "ws-rom", "expectation": "frozen"}}
        (self.output / "versions.json").write_text(json.dumps(self.versions))

        codec = {"codec": "lz-rle-7f", "consumed": 4, "require_fit": True,
                 "provenance": "synthetic fixture", "evidence": "synthetic"}
        for kind, data in (("raw", b"A"), ("stored", b"\x80A\x7f\xff"),
                           ("encoded", b"\x80A\x7f\xff")):
            source = root / kind
            source.write_bytes(data)
            codec[kind + "_path"] = str(source)
            codec[kind + "_sha256"] = hashlib.sha256(data).hexdigest()
        self.codec_manifest = root / "codec.json"
        self.codec_manifest.write_text(json.dumps({"records": [codec], "oracle_sources": {}}))

        tile = root / "sample.tiles"
        tile.write_bytes(bytes(16))
        self.gfx_manifest = root / "graphics.json"
        self.gfx_manifest.write_text(json.dumps({"samples": [{
            "name": tile.name, "path": str(tile),
            "args": {"operation": "tiles", "tile_format": "2bpp"},
            "expect": {"sha256": hashlib.sha256(tile.read_bytes()).hexdigest()},
            "reference_sha256": "a" * 64,
        }]}))

        wave = root / "sample.raw"
        wave.write_bytes(bytes(range(16)))
        reference = root / "oracle.txt"
        reference.write_text("independent synthetic sound expectation\n")
        self.sound_manifest = root / "sound.json"
        self.sound_manifest.write_text(json.dumps({"samples": [{
            "name": wave.name, "source": str(wave),
            "args": {"references": [{
                "path": str(reference),
                "sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
            }]},
            "expect": {"sha256": hashlib.sha256(wave.read_bytes()).hexdigest()},
        }]}))

    def run_cli(self, *selection):
        environment = dict(os.environ)
        environment.pop("SWANKIT_PROGRESS_LOG", None)
        return subprocess.run(
            [sys.executable, str(SCRIPT), *map(str, selection), "--output", str(self.output)],
            capture_output=True, text=True, env=environment, check=False,
        )

    def frozen_files(self):
        return {path.relative_to(self.output): path.read_bytes()
                for path in self.output.rglob("*") if path.is_file()}

    def enroll(self, order):
        original = self.frozen_files()
        for option, manifest in order:
            result = self.run_cli(option, manifest)
            self.assertEqual(result.returncode, 0, result.stderr)
        frozen = self.frozen_files()
        for path, data in original.items():
            if path.name != "versions.json":
                self.assertEqual(frozen[path], data)
        versions = json.loads((self.output / "versions.json").read_text())
        self.assertEqual(versions["existing"], self.versions["existing"])
        formats = {"--codec-manifest": "ws-compress", "--gfx-manifest": "ws-gfx",
                   "--sound-manifest": "ws-sound"}
        for option, _ in order:
            format_name = formats[option]
            spec = json.loads((self.output / format_name / "spec.json").read_text())
            self.assertEqual(spec["format"], format_name)
            self.assertTrue(spec["samples"])
        for option, manifest in order:
            result = self.run_cli(option, manifest)
            self.assertEqual(result.returncode, 1, result.stderr)
            refusal = "cannot be overwritten" if option == "--sound-manifest" else "never be overwritten"
            self.assertIn(refusal, result.stderr)
            self.assertEqual(self.frozen_files(), frozen)

    def test_codec_then_graphics_preserves_specs_and_refuses_overwrite(self):
        self.enroll((("--codec-manifest", self.codec_manifest),
                          ("--gfx-manifest", self.gfx_manifest)))

    def test_graphics_then_codec_preserves_specs_and_refuses_overwrite(self):
        self.enroll((("--gfx-manifest", self.gfx_manifest),
                          ("--codec-manifest", self.codec_manifest)))

    def test_codec_graphics_sound_preserves_specs_and_refuses_overwrite(self):
        self.enroll((("--codec-manifest", self.codec_manifest),
                          ("--gfx-manifest", self.gfx_manifest),
                          ("--sound-manifest", self.sound_manifest)))

    def test_sound_graphics_codec_preserves_specs_and_refuses_overwrite(self):
        self.enroll((("--sound-manifest", self.sound_manifest),
                          ("--gfx-manifest", self.gfx_manifest),
                          ("--codec-manifest", self.codec_manifest)))

    def test_selection_is_required_and_mutually_exclusive_before_writing(self):
        frozen = self.frozen_files()
        options = (("--titles", self.output), ("--codec-manifest", self.codec_manifest),
                   ("--gfx-manifest", self.gfx_manifest),
                   ("--sound-manifest", self.sound_manifest))
        selections = [()]
        selections.extend(options[a] + options[b]
                          for a in range(len(options)) for b in range(a + 1, len(options)))
        for selection in selections:
            with self.subTest(selection=selection):
                result = self.run_cli(*selection)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(self.frozen_files(), frozen)

    def test_sound_directory_selections_refuse_overwrite(self):
        for directory in ("ws-sound", "ws-sound-extra", "ws-sound-captures"):
            with self.subTest(directory=directory):
                before = self.frozen_files()
                selection = ("--sound-manifest", self.sound_manifest,
                             "--sound-spec-dir", directory)
                result = self.run_cli(*selection)
                self.assertEqual(result.returncode, 0, result.stderr)
                spec = json.loads((self.output / directory / "spec.json").read_text())
                self.assertEqual(spec["format"], "ws-sound")
                frozen = self.frozen_files()
                for path, data in before.items():
                    self.assertEqual(frozen[path], data)
                result = self.run_cli(*selection)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("cannot be overwritten", result.stderr)
                self.assertEqual(self.frozen_files(), frozen)

    def test_display_directory_selection_refuses_overwrite(self):
        selection = ("--gfx-manifest", self.gfx_manifest, "--gfx-format", "ws-gfx-display")
        result = self.run_cli(*selection)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.output / "ws-gfx-display" / "spec.json").is_file())
        self.assertFalse((self.output / "ws-gfx").exists())
        frozen = self.frozen_files()
        result = self.run_cli(*selection)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("never be overwritten", result.stderr)
        self.assertEqual(self.frozen_files(), frozen)
