"""Synthetic package integrity and complete offline-guide checks."""
import base64
import hashlib
import importlib.util
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("package_release", ROOT / "tools/package-release.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class ReleaseTests(unittest.TestCase):
    def test_offline_guide_closure(self):
        payload = PACKAGE.guide_payload(ROOT)
        required = {"README.md", "docs/cli.md", "docs/distribution.md", "docs/link.md",
                    "docs/python.md", "docs/save-definition.schema.json", "docs/asset-map.schema.json",
                    "crates/swankit-ffi/README.md", "crates/ws-text/data/README.md",
                    "crates/ws-wondergate-server/data/services.json", *PACKAGE.NOTICES}
        self.assertTrue(required <= payload.keys())
        PACKAGE.validate_links(payload)
        for missing in ["docs/link.md", "docs/asset-map.schema.json", "crates/swankit-ffi/README.md"]:
            with self.subTest(missing=missing), self.assertRaisesRegex(ValueError, "missing offline"):
                PACKAGE.validate_links({k: v for k, v in payload.items() if k != missing})
        with self.assertRaisesRegex(ValueError, "escapes packet"):
            PACKAGE.validate_links({"README.md": b"[bad](../private.md)"})

    def test_transitive_linked_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "docs").mkdir()
            (root / "crates").mkdir()
            for name in ["CONTRIBUTING.md", *PACKAGE.NOTICES]:
                (root / name).write_text("text")
            (root / "README.md").write_text("[guide](docs/guide.md)")
            (root / "docs/guide.md").write_text("[nested](../nested/more.md)")
            (root / "nested").mkdir()
            (root / "nested/more.md").write_text("[definition](definition.json)")
            (root / "nested/definition.json").write_text("{}")
            payload = PACKAGE.guide_payload(root)
            self.assertEqual(payload["nested/definition.json"], b"{}")
            (root / "nested/definition.json").unlink()
            with self.assertRaisesRegex(ValueError, "missing or unsafe"):
                PACKAGE.guide_payload(root)

    def test_generated_and_hidden_readmes_are_not_offline_guides(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ["README.md", "CONTRIBUTING.md", *PACKAGE.NOTICES]:
                (root / name).write_text("synthetic public guide")
            expected = {}
            for name in ["docs/guide.md", "docs/definition.json", "crates/example/README.md"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("{}" if name.endswith(".json") else "public guide")
                expected[name] = path.read_bytes()
            hidden = ["crates/example/.pytest_cache/README.md", "docs/.cache/hidden.md",
                      "crates/example/target/README.md", "crates/example/__pycache__/README.md",
                      "crates/example/release-licenses/dep/README.md"]
            for name in hidden:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("cache content must not be distributed")
            payload = PACKAGE.guide_payload(root)
            self.assertTrue(expected.keys() <= payload.keys())
            self.assertFalse(set(hidden) & payload.keys())
            for name, raw in expected.items():
                self.assertEqual(payload[name], raw)

    def test_explicit_hidden_or_generated_guide_links_are_refused(self):
        for target in [".git/config", "%2egit/config", "docs/.cache/hidden.md",
                       "crates/example/.pytest_cache/README.md", "crates/example/target/README.md"]:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for name in ["CONTRIBUTING.md", *PACKAGE.NOTICES]:
                    (root / name).write_text("public guide")
                from urllib.parse import unquote
                path = root / unquote(target)
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("must never enter the packet")
                (root / "README.md").write_text("[private](" + target + ")")
                with self.subTest(target=target), self.assertRaisesRegex(ValueError, "hidden or generated"):
                    PACKAGE.guide_payload(root)

    def test_encoded_absolute_links_are_refused_before_reading(self):
        from urllib.parse import quote
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "docs").mkdir()
            (root / "crates").mkdir()
            for name in ["CONTRIBUTING.md", *PACKAGE.NOTICES]:
                (root / name).write_text("text")
            target = root / "unlinked.txt"
            target.write_text("must not enter the payload")
            (root / "README.md").write_text("[absolute](" + quote(str(target), safe="") + ")")
            with self.assertRaisesRegex(ValueError, "absolute guide link"):
                PACKAGE.guide_payload(root)

    def test_repeatable_archive_and_modes(self):
        payload = {"swankit": b"binary", "README.md": b"guide", "docs/cli.md": b"syntax"}
        archive = PACKAGE.archive_bytes(payload, "release")
        self.assertEqual(archive, PACKAGE.archive_bytes(payload, "release"))
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as packed:
            self.assertEqual(set(packed.getnames()), {"release/" + name for name in payload})
            for name, data in payload.items():
                member = packed.getmember("release/" + name)
                self.assertEqual(member.mode, 0o755 if name == "swankit" else 0o644)
                self.assertEqual(packed.extractfile(member).read(), data)

    def test_wheel_integrity_and_legal_checks(self):
        members = {"swankit/_native.abi3.so": Path(sys.executable).resolve().read_bytes(),
                   "swankit-0.1.0.dist-info/METADATA": b"Version: 0.1.0\n",
                   "swankit-0.1.0.dist-info/WHEEL": b"Tag: cp311-abi3-manylinux_2_34_x86_64\n"}
        members.update({"swankit-0.1.0.dist-info/licenses/" + name: (ROOT / name).read_bytes()
                        for name in PACKAGE.NOTICES})
        record = "swankit-0.1.0.dist-info/RECORD"
        records = []
        for name, data in members.items():
            digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).decode().rstrip("=")
            records.append(f"{name},sha256={digest},{len(data)}\n")
        members[record] = ("".join(records) + record + ",,\n").encode()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for case in ["valid", "tampered", "wrong-notice"]:
                variant = dict(members)
                if case == "tampered":
                    variant["swankit/_native.abi3.so"] += b"tampered"
                if case == "wrong-notice":
                    # A correctly hashed but unauthoritative notice still fails.
                    name = "swankit-0.1.0.dist-info/licenses/NOTICE"
                    variant[name] = b"different notice"
                    old = next(row for row in records if row.startswith(name + ","))
                    digest = base64.urlsafe_b64encode(hashlib.sha256(variant[name]).digest()).decode().rstrip("=")
                    variant[record] = variant[record].replace(old.encode(), f"{name},sha256={digest},{len(variant[name])}\n".encode())
                path = root / (case + ".whl")
                with zipfile.ZipFile(path, "w") as wheel:
                    for name, data in variant.items():
                        wheel.writestr(name, data)
                if case == "valid":
                    self.assertIn("glibc_symbol_minimum", PACKAGE.wheel_audit(path, ROOT, root)["elf"])
                else:
                    with self.subTest(case=case), self.assertRaises(ValueError):
                        PACKAGE.wheel_audit(path, ROOT, root)


    def test_wheel_dependency_notices_match_authoritative_bundle(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bundle = root / "bundle"
            (bundle / "synthetic-0.1.0").mkdir(parents=True)
            (bundle / "DEPENDENCIES.json").write_text("{}")
            (bundle / "synthetic-0.1.0/LICENSE").write_text("Copyright synthetic\nSynthetic terms\n")
            prefix = "swankit-0.1.0.dist-info/licenses/release-licenses/"
            for case in ["valid", "missing", "changed"]:
                members = {"swankit/_native.abi3.so": Path(sys.executable).resolve().read_bytes(),
                           "swankit-0.1.0.dist-info/METADATA": b"Version: 0.1.0\n",
                           "swankit-0.1.0.dist-info/WHEEL": b"Tag: cp311-abi3-manylinux_2_34_x86_64\n"}
                members.update({"swankit-0.1.0.dist-info/licenses/" + n: (ROOT / n).read_bytes()
                                for n in PACKAGE.NOTICES})
                members.update({prefix + p.relative_to(bundle).as_posix(): p.read_bytes()
                                for p in bundle.rglob("*") if p.is_file()})
                key = prefix + "synthetic-0.1.0/LICENSE"
                if case == "missing":
                    del members[key]
                elif case == "changed":
                    members[key] = b"changed but correctly hashed"
                record = "swankit-0.1.0.dist-info/RECORD"
                rows = []
                for name, raw in members.items():
                    checksum = base64.urlsafe_b64encode(hashlib.sha256(raw).digest()).decode().rstrip("=")
                    rows.append(f"{name},sha256={checksum},{len(raw)}\n")
                members[record] = ("".join(rows) + record + ",,\n").encode()
                path = root / (case + ".whl")
                with zipfile.ZipFile(path, "w") as wheel:
                    for name, raw in members.items():
                        wheel.writestr(name, raw)
                if case == "valid":
                    PACKAGE.wheel_audit(path, ROOT, root, bundle)
                else:
                    with self.subTest(case=case), self.assertRaisesRegex(ValueError, "dependency notices"):
                        PACKAGE.wheel_audit(path, ROOT, root, bundle)


if __name__ == "__main__":
    unittest.main()
