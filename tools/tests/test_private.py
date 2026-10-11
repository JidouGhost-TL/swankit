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

    def test_ordinary_cursor_words_are_allowed(self):
        for text in [b"option cursor", b"menu cursor", b"cursor X", b"battle save: cursor row"]:
            self.assertEqual(module.violations("notes.md", text, "100644", "sample-user"), [])

    def test_tool_names_and_specific_editor_references_are_rejected(self):
        names = ["clau" + "de", "co" + "dex", "mu" + "se", "anthro" + "pic",
                 "open" + "ai", "gemi" + "ni", "copi" + "lot",
                 "cur" + "sor.sh", "Cur" + "sor IDE"]
        for name in names:
            with self.subTest(name=name):
                self.assertIn("assistant product identifier",
                              module.violations("notes.md", name.encode(), "100644", "sample-user"))

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

class ReviewPrivacyGaps(unittest.TestCase):
    def test_scanner_source_is_not_exempt_from_content_rules(self):
        leaked = b"# contact " + b"someone" + b"@" + b"example.net"
        self.assertIn("non-repository email",module.violations("tools/check-private.py",SCANNER.read_bytes()+leaked,"100644","sample-user"))

    def test_additional_storage_paths_are_rejected(self):
        paths = ["/"+base+"/private/data" for base in ("srv","opt","root")]
        paths += ["\\"*2+"nas"+"\\share\\roms", "sm"+"b://nas/share", "nf"+"s://nas/share"]
        for path in paths:
            with self.subTest(path=path):
                self.assertIn("machine path",module.violations("notes.md",path.encode(),"100644","sample-user"))


if __name__ == "__main__":
    unittest.main()
