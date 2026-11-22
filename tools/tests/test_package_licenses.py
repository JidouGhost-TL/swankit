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
        self.assertEqual(metadata["project"]["license-files"], expected + ["release-licenses/**/*"])
        for name in expected:
            with self.subTest(name=name):
                self.assertEqual((root / name).read_bytes(), (package / name).read_bytes())

    def test_all_source_crates_include_authoritative_notices(self):
        root = Path(__file__).resolve().parents[2]
        workspace = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]
        self.assertFalse(workspace["package"]["publish"])
        public_libraries = {
            "swankit", "ws-rom", "ws-compress", "ws-gfx", "ws-text", "ws-sound",
            "ws-patch", "ws-save", "ws-saves", "ws-assets", "ws-discover",
        }
        for member in workspace["members"]:
            package = root / member
            metadata = tomllib.loads((package / "Cargo.toml").read_text())
            with self.subTest(package=member):
                package_metadata = metadata["package"]
                expected_policy = ["crates-io"] if package_metadata["name"] in public_libraries else {"workspace": True}
                self.assertEqual(package_metadata["publish"], expected_policy)
                if expected_policy == ["crates-io"]:
                    self.assertIn("/src/**", package_metadata["include"])
            for name in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE"]:
                with self.subTest(package=member, notice=name):
                    self.assertFalse((package / name).is_symlink())
                    self.assertEqual((root / name).read_bytes(), (package / name).read_bytes())
