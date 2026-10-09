#!/usr/bin/env python3
"""Audit complete indexed blobs, optionally including current working files."""
import argparse
import getpass
from pathlib import Path
import re
import subprocess
import sys


IDENTITY_EMAIL = "hello@jidoughost.com"
DENY_PATH = re.compile(
    r"(?:^|/)(?:target|corpus|research|reports|internal-docs|internal-tools)(?:/|$)",
    re.I,
)
MACHINE_PATH = re.compile(
    r"/(?:home|Users|mnt|media|var/tmp|tmp)/|[A-Z]:[\\/](?:Users|Documents)[\\/]|file://|~[/\\]",
    re.I,
)
TOOL_NAME = re.compile(
    r"\b(?:" + "|".join(("co" + "dex", "clau" + "de", "chat" + "gpt",
                           "open" + "ai", "copi" + "lot", "gemi" + "ni",
                           "cur" + "sor", "mu" + "se", "gpt" + r"[- ]?\d+")) + r")\b",
    re.I,
)
EMAIL = re.compile(r"[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}", re.I)
CORPUS_EXTENSIONS = {
    ".ws", ".wsc", ".pc2", ".rom", ".bin", ".sav", ".srm", ".eep",
    ".zip", ".7z", ".rar", ".gz", ".xz", ".tar",
}
SECRET = re.compile(r"-----BEGIN (?:OPENSSH|RSA|EC) PRIVATE KEY-----|\b(?:sk-proj-|ghp_)[A-Za-z0-9]+")


def violations(path, blob, mode, username):
    reasons = []
    if (DENY_PATH.search(path) or Path(path).suffix.lower() in CORPUS_EXTENSIONS
            or Path(path).name in {"spec.json", "rom-facts.json", "versions.json", "disagreements.json"}):
        reasons.append("corpus/generated path")
    if mode == "120000":
        reasons.append("symlink in repository")
    try:
        content = blob.decode("utf-8")
    except UnicodeDecodeError:
        return reasons + ["binary content"]
    if "\0" in content:
        reasons.append("binary content")
    # As in the upstream gate, the scanner's rule declarations are self-exempt.
    if path == "tools/check-private.py":
        return reasons
    combined = path + "\n" + content
    if MACHINE_PATH.search(combined):
        reasons.append("machine path")
    if TOOL_NAME.search(combined):
        reasons.append("assistant product identifier")
    if username and username != "jidoughost" and re.search(r"\b" + re.escape(username) + r"\b", combined, re.I):
        reasons.append("workstation username")
    if any(email.lower() != IDENTITY_EMAIL for email in EMAIL.findall(combined)):
        reasons.append("non-repository email")
    if SECRET.search(combined):
        reasons.append("credential marker")
    return reasons


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worktree", action="store_true", help="also scan working and untracked files")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    username = getpass.getuser()
    failures = []
    checked = 0
    for entry in filter(None, git(root, "ls-files", "--stage", "-z").split(b"\0")):
        metadata, raw_path = entry.split(b"\t", 1)
        mode, object_id, stage = metadata.decode().split()
        path = raw_path.decode("utf-8", "surrogateescape")
        if stage != "0":
            failures.append(f"{path}: unresolved index stage")
            continue
        blob = git(root, "cat-file", "blob", object_id)
        failures.extend(f"{path}: {reason}" for reason in violations(path, blob, mode, username))
        checked += 1
    if args.worktree:
        paths = git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z")
        for raw_path in sorted(set(filter(None, paths.split(b"\0")))):
            path = raw_path.decode("utf-8", "surrogateescape")
            file = root / path
            if not file.exists() and not file.is_symlink():
                continue
            blob = str(file.readlink()).encode() if file.is_symlink() else file.read_bytes()
            mode = "120000" if file.is_symlink() else "100644"
            failures.extend(f"{path}: {reason}" for reason in violations(path, blob, mode, username))
            checked += 1
    if failures:
        print("\n".join(sorted(set(failures))), file=sys.stderr)
        return 1
    if not checked:
        print("Private boundary: no files inspected", file=sys.stderr)
        return 1
    print(f"Private boundary: passed ({checked} blobs/files inspected)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
