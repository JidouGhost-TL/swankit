"""Ensure package metadata preserves licences of ports and linked binaries."""
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]

class LicenseMetadata(unittest.TestCase):
    def test_mit_ports_and_linking_binary_preserve_mit_condition(self):
        for crate in ("ws-wondergate", "ws-wondergate-server", "ws-link", "swankit-cli"):
            manifest = tomllib.loads((ROOT / "crates" / crate / "Cargo.toml").read_text())
            self.assertEqual(manifest["package"]["license"], "MIT AND (MIT OR Apache-2.0)", crate)
