"""Exercise container command construction without invoking a container daemon."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ContainerWrapperTests(unittest.TestCase):
    def test_ordinary_modes_need_no_sibling(self):
        for mode in ["wheel", "--ci"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                root = base / "source"
                (root / "tools").mkdir(parents=True)
                shutil.copyfile(ROOT / "tools/python-container.sh", root / "tools/python-container.sh")
                subprocess.run(["git", "init", "--quiet", str(root)], check=True)
                commands = base / "commands"
                commands.mkdir()
                docker = commands / "docker"
                docker.write_text('#!/bin/sh\nprintf "%s\\0" "$@" >> "$CAPTURE"\n')
                docker.chmod(0o755)
                capture = base / "capture"
                environment = {**os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                               "CAPTURE": str(capture), "SWANKIT_PYTHON_ARTIFACTS": str(base / "artifacts"),
                               "FORMATKIT_DIR": str(base / "missing-sibling"),
                               "XDG_RUNTIME_DIR": str(base)}
                result = subprocess.run(["bash", str(root / "tools/python-container.sh"), mode],
                                        env=environment, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                arguments = capture.read_bytes().decode().split("\0")
                self.assertNotIn(str(base / "missing-sibling"), "\n".join(arguments))
                self.assertFalse(any("formatkit:ro" in argument for argument in arguments))
                self.assertIn("FORMATKIT_DIR=", arguments)
                self.assertIn(mode, arguments)

    def test_corpus_mode_requires_explicit_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            environment = {**os.environ, "SWANKIT_PYTHON_ARTIFACTS": str(base / "artifacts"),
                           "FORMATKIT_DIR": str(base / "missing-runner")}
            # A missing runner must fail before attempting a build or a mount.
            commands = base / "commands"
            commands.mkdir()
            docker = commands / "docker"
            docker.write_text('#!/bin/sh\nexit 99\n')
            docker.chmod(0o755)
            environment["PATH"] = str(commands) + os.pathsep + os.environ["PATH"]
            result = subprocess.run(["bash", str(ROOT / "tools/python-container.sh"), "--ci-corpus"],
                                    env=environment, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotEqual(result.returncode, 99)
            self.assertIn("missing-runner", result.stderr)

    def test_corpus_runner_is_mapped_to_container_path(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            runner = base / "runner"
            runner.mkdir()
            corpus = base / "corpus"
            corpus.mkdir()
            commands = base / "commands"
            commands.mkdir()
            capture = base / "capture"
            docker = commands / "docker"
            docker.write_text('#!/bin/sh\nprintf "%s\\0" "$@" >> "$CAPTURE"\n')
            docker.chmod(0o755)
            environment = {**os.environ, "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                           "CAPTURE": str(capture), "SWANKIT_PYTHON_ARTIFACTS": str(base / "artifacts"),
                           "FORMATKIT_DIR": str(runner), "FORMATKIT_CORPUS_DIR": str(corpus),
                           "SWANKIT_CORPUS_DATA_ROOTS": "", "XDG_RUNTIME_DIR": str(base)}
            result = subprocess.run(["bash", str(ROOT / "tools/python-container.sh"), "--ci-corpus"],
                                    env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            arguments = capture.read_bytes().decode().split("\0")
            self.assertIn(str(runner) + ":/workspace/formatkit:ro", arguments)
            self.assertIn("FORMATKIT_DIR=/workspace/formatkit", arguments)
            self.assertIn("FORMATKIT_CORPUS_DIR=" + str(corpus), arguments)


if __name__ == "__main__":
    unittest.main()
