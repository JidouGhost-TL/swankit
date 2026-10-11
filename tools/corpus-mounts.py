#!/usr/bin/env python3
"""List external data directories referenced by staged specs and symlinks."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
directories = set()
for value in sys.argv[2:]:
    data_root = Path(value).resolve(strict=True)
    if not data_root.is_dir():
        raise SystemExit(f"data root is not a directory: {data_root}")
    directories.add(data_root)

def admit(path):
    if path.is_absolute() and path.is_file():
        parent = path.resolve().parent
        if parent != root and root not in parent.parents:
            directories.add(parent)

def walk(value):
    if isinstance(value,str):
        admit(Path(value))
    elif isinstance(value,dict):
        for item in value.values():
            walk(item)
    elif isinstance(value,list):
        for item in value:
            walk(item)

for path in root.rglob("*"):
    if path.is_symlink():
        admit(path.resolve())
    elif path.suffix == ".json":
        walk(json.loads(path.read_text()))
for directory in sorted(directories):
    if not any(parent in directories for parent in directory.parents):
        sys.stdout.buffer.write(str(directory).encode()+b"\0")
