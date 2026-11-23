"""Dependency notices are exact upstream evidence, not inferred license terms."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("dependency_licenses", Path(__file__).resolve().parents[1] / "collect-dependency-licenses.py")
LICENSES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LICENSES)


class LicenseCollectionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "crate"
        (self.source / "LICENSES").mkdir(parents=True)
        self.notice = b"Copyright synthetic\nPermission is granted by this synthetic test.\n"
        (self.source / "LICENSES/MIT.txt").write_bytes(self.notice)
        (self.source / "Cargo.toml").write_text('[package]\nname="synthetic"\nversion="0.1.0"\n')
        self.cargo = self.root / "cargo"
        cache = self.cargo / "registry/cache/index"
        cache.mkdir(parents=True)
        raw = io.BytesIO()
        with tarfile.open(fileobj=raw, mode="w:gz") as archive:
            for path in self.source.rglob("*"):
                if path.is_file():
                    item = tarfile.TarInfo("synthetic-0.1.0/" + path.relative_to(self.source).as_posix())
                    item.size = path.stat().st_size
                    archive.addfile(item, io.BytesIO(path.read_bytes()))
        self.archive = cache / "synthetic-0.1.0.crate"
        self.archive.write_bytes(raw.getvalue())
        self.lock = ('[[package]]\nname="synthetic"\nversion="0.1.0"\n'
                     'source="registry+https://github.com/rust-lang/crates.io-index"\n'
                     'checksum="' + hashlib.sha256(raw.getvalue()).hexdigest() + '"\n').encode()
        self.metadata = {"packages": [{"name": "synthetic", "version": "0.1.0", "license": "MIT",
                       "license_file": None, "source": "registry+https://github.com/rust-lang/crates.io-index",
                       "manifest_path": str(self.source / "Cargo.toml")} ]}
        self.output = self.root / "bundle"

    def collect(self):
        return LICENSES.collect(self.metadata, self.lock, self.cargo, self.output)

    def test_actual_archive_bytes_and_nested_notices_are_preserved(self):
        self.assertEqual(self.collect()["notice_files"], 1)
        inventory = LICENSES.verify_bundle(self.output, LICENSES.sha(self.lock))
        path = inventory["packages"][0]["licenses"][0]["bundle_path"]
        self.assertEqual((self.output / path).read_bytes(), self.notice)
        self.assertEqual(self.collect()["packages"], 1)
        self.assertNotIn(str(self.root), (self.output / "DEPENDENCIES.json").read_text())

    def test_missing_or_changed_published_license_refuses_before_output(self):
        path = self.source / "LICENSES/MIT.txt"
        path.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "published archive"):
            self.collect()
        self.assertFalse(self.output.exists())
        path.unlink()
        with self.assertRaisesRegex(ValueError, "missing declared"):
            self.collect()
        self.assertFalse(self.output.exists())

    def test_changed_archive_checksum_refuses(self):
        self.archive.write_bytes(b"changed archive")
        with self.assertRaisesRegex(ValueError, "archive checksum"):
            self.collect()
        self.assertFalse(self.output.exists())

    def test_patch_notice_bundle_requires_the_matching_release_source(self):
        self.collect()
        inventory_path = self.output / "DEPENDENCIES.json"
        inventory = json.loads(inventory_path.read_bytes())
        package = inventory["packages"][0]
        version = "0.1.4"
        pin = dict(LICENSES.PATCH_ARCHIVES[version], notices={"NOTICE": LICENSES.sha(self.notice)})
        with patch.dict(LICENSES.PATCH_ARCHIVES, {version: pin}):
            package.update(name="formatkit-patch", version=version, registry_checksum=pin["patch_checksum"],
                           notice_source=LICENSES.patch_notice_source(version))
            package["licenses"][0]["source_path"] = "NOTICE"
            inventory_path.write_text(json.dumps(inventory))
            LICENSES.verify_bundle(self.output)
            package["notice_source"] = LICENSES.patch_notice_source("0.1.1")
            inventory_path.write_text(json.dumps(inventory))
            with self.assertRaisesRegex(ValueError, "Formatkit patch notice provenance"):
                LICENSES.verify_bundle(self.output)

    def test_missing_modified_and_escaping_bundle_refuse(self):
        self.collect()
        inventory_path = self.output / "DEPENDENCIES.json"
        original = inventory_path.read_bytes()
        inventory = json.loads(original)
        path = self.output / inventory["packages"][0]["licenses"][0]["bundle_path"]
        path.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "notice bytes changed"):
            LICENSES.verify_bundle(self.output)
        with self.assertRaisesRegex(ValueError, "notice bytes changed"):
            self.collect()
        self.assertEqual(path.read_bytes(), b"changed")
        path.unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            LICENSES.verify_bundle(self.output)
        inventory["packages"][0]["licenses"][0]["bundle_path"] = "../escape"
        inventory_path.write_text(json.dumps(inventory))
        with self.assertRaisesRegex(ValueError, "unsafe"):
            LICENSES.verify_bundle(self.output)


if __name__ == "__main__":
    unittest.main()
