"""Synthetic regression checks for the complete-index privacy boundary."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


SCANNER = Path(__file__).resolve().parents[1] / "check-private.py"
spec = importlib.util.spec_from_file_location("private_gate", SCANNER)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class Rules(unittest.TestCase):
    def test_all_disclosure_classes(self):
        cases = [
            ("notes.md", ("/" + "home/" + "sample/project").encode(), "machine path"),
            ("notes.md", ("/" + "var/" + "tmp/" + "sample").encode(), "machine path"),
            ("notes.md", b"sample-user", "workstation username"),
            ("notes.md", b"someone" + b"@" + b"example.net", "non-repository email"),
            ("notes.md", ("co" + "dex").encode(), "assistant product identifier"),
            ("sample.ws", b"plain text", "corpus/generated path"),
            ("spec.json", b"{}", "corpus/generated path"),
            ("data.txt", b"binary\0bytes", "binary content"),
            ("data.txt", b"\xff", "binary content"),
        ]
        for path, blob, expected in cases:
            with self.subTest(path=path, expected=expected):
                self.assertIn(expected, module.violations(path, blob, "100644", "sample-user"))

    def test_identity_and_neutral_source_are_allowed(self):
        self.assertEqual(module.violations("README.md", b"jidoughost <hello@jidoughost.com>", "100644", "sample-user"), [])
        self.assertEqual(module.violations("src/lib.rs", b"const SYNTHETIC: [u8; 2] = [1, 2];", "100644", "sample-user"), [])

    def test_symlinks_cannot_hide_external_content(self):
        self.assertIn("symlink in repository", module.violations("link", b"../sample", "120000", "sample-user"))

    def test_indexed_blob_is_checked_even_when_working_file_is_clean(self):
        with tempfile.TemporaryDirectory(prefix="swankit-privacy-") as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "--initial-branch=main", str(root)], check=True, capture_output=True)
            (root / "tools").mkdir()
            (root / "tools/check-private.py").write_bytes(SCANNER.read_bytes())
            notes = root / "notes.md"
            notes.write_text("co" + "dex")
            subprocess.run(["git", "add", "--", "tools/check-private.py", "notes.md"], cwd=root, check=True)
            notes.write_text("neutral working text")
            rejected = subprocess.run(["python3", "tools/check-private.py"], cwd=root, capture_output=True, text=True)
            self.assertEqual(rejected.returncode, 1)
            self.assertIn("notes.md: assistant product identifier", rejected.stderr)
            subprocess.run(["git", "add", "--", "notes.md"], cwd=root, check=True)
            accepted = subprocess.run(["python3", "tools/check-private.py"], cwd=root, capture_output=True, text=True)
            self.assertEqual(accepted.returncode, 0, accepted.stderr)


if __name__ == "__main__":
    unittest.main()
