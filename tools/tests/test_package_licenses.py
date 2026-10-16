"""Keep distribution notices identical to the workspace originals."""
from pathlib import Path
import tomllib
import unittest


class PackageLicenseTests(unittest.TestCase):
    def test_python_distribution_includes_authoritative_notices(self):
        root = Path(__file__).resolve().parents[2]
        package = root / "crates" / "swankit-py"
        expected = ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE"]
        metadata = tomllib.loads((package / "pyproject.toml").read_text())
        self.assertEqual(metadata["project"]["license-files"], expected)
        for name in expected:
            with self.subTest(name=name):
                self.assertEqual((root / name).read_bytes(), (package / name).read_bytes())
