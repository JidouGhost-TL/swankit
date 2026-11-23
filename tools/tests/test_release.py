"""Release approval boundaries use synthetic packets, never private inputs."""
import copy
import hashlib
import importlib.util
import io
import json
import http.server
from pathlib import Path
import tarfile
import tempfile
import unittest
import threading
import urllib.request
import zipfile
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parents[1] / "release.py")
RELEASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE)

REPOSITORY = "JidouGhost-TL/swankit"


class WorkflowIdentityTests(unittest.TestCase):
    def setUp(self):
        self.commit = "a" * 40
        self.env = {"GITHUB_REPOSITORY": REPOSITORY, "GITHUB_SERVER_URL": "https://github.com",
                    "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REF": "refs/heads/main",
                    "GITHUB_SHA": self.commit, "GITHUB_WORKFLOW_SHA": self.commit}

    def test_canonical_candidate_and_publication_context(self):
        for workflow in (RELEASE.CANDIDATE_WORKFLOW, RELEASE.PUBLICATION_WORKFLOW):
            with self.subTest(workflow=workflow):
                env = dict(self.env, GITHUB_WORKFLOW_REF=f"{REPOSITORY}/{workflow}@refs/heads/main")
                source = self.commit if workflow == RELEASE.CANDIDATE_WORKFLOW else None
                RELEASE.validate_workflow_context(env, workflow, self.commit, source)

    def test_refuse_fork_wrong_ref_workflow_and_source(self):
        workflow = RELEASE.CANDIDATE_WORKFLOW
        env = dict(self.env, GITHUB_WORKFLOW_REF=f"{REPOSITORY}/{workflow}@refs/heads/main")
        alternatives = {
            "GITHUB_REPOSITORY": "other/swankit",
            "GITHUB_SERVER_URL": "https://example.invalid",
            "GITHUB_EVENT_NAME": "pull_request_target",
            "GITHUB_REF": "refs/tags/main",
            "GITHUB_WORKFLOW_REF": f"{REPOSITORY}/{RELEASE.PUBLICATION_WORKFLOW}@refs/heads/main",
            "GITHUB_SHA": "b" * 40,
            "GITHUB_WORKFLOW_SHA": "b" * 40,
        }
        for key, value in alternatives.items():
            with self.subTest(key=key), self.assertRaises(ValueError):
                RELEASE.validate_workflow_context(dict(env, **{key: value}), workflow, self.commit, self.commit)
        for checkout, source in ((self.commit, None), (self.commit, "b" * 40), ("a" * 39, self.commit)):
            with self.subTest(checkout=checkout, source=source), self.assertRaises(ValueError):
                RELEASE.validate_workflow_context(env, workflow, checkout, source)

    def test_api_client_rejects_other_repositories_before_any_request(self):
        with self.assertRaisesRegex(ValueError, "unexpected release repository"):
            RELEASE.GitHub("other/swankit", "synthetic")


class ApprovalTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.packet = self.root / "candidate" / "packet"
        self.packet.mkdir(parents=True)
        self.commit = "a" * 40
        self.lock = "b" * 64
        license_text = b"Synthetic dependency license text"
        inventory = {"schema": "swankit-dependency-licenses/1",
                     "scope": "all-resolved-packages-including-build-dev-and-target-specific",
                     "lock_sha256": self.lock, "packages": [{"name": "synthetic", "version": "0.1.0",
                     "declared_license": "MIT", "source": "workspace", "registry_checksum": None,
                     "licenses": [{"source_path": "LICENSE", "bundle_path": "synthetic-0.1.0/LICENSE",
                                  "bytes": len(license_text), "sha256": RELEASE.digest(license_text)}]}]}
        registry_version = RELEASE.FORMATKIT_VERSION
        registry_source = "registry+https://github.com/rust-lang/crates.io-index"
        checksum = "a" * 64
        notices = {"synthetic-0.1.0/LICENSE": license_text}
        notice_tool = RELEASE.DEPENDENCY_LICENSES
        pin = {**notice_tool.PATCH_ARCHIVES[registry_version], "patch_checksum": checksum,
               "notices": {"LICENSE": RELEASE.digest(license_text)}}
        patch_archives = patch.dict(notice_tool.PATCH_ARCHIVES, {registry_version: pin})
        patch_archives.start()
        self.addCleanup(patch_archives.stop)
        for name in sorted(RELEASE.FORMATKIT):
            path = f"{name}-{registry_version}/LICENSE"
            notices[path] = license_text
            package = {"name": name, "version": registry_version, "declared_license": "MIT",
                       "source": registry_source, "registry_checksum": checksum,
                       "licenses": [{"source_path": "LICENSE", "bundle_path": path,
                                     "bytes": len(license_text), "sha256": RELEASE.digest(license_text)}]}
            if name == "formatkit-patch":
                package["notice_source"] = notice_tool.patch_notice_source(registry_version)
            inventory["packages"].append(package)
        inventory_raw = json.dumps(inventory).encode()
        wheel = io.BytesIO()
        with zipfile.ZipFile(wheel, "w") as out:
            out.writestr("swankit/_native.abi3.so", b"synthetic extension")
            out.writestr("swankit/__init__.py", b"# synthetic")
            out.writestr("swankit-0.1.0.dist-info/RECORD", "")
            out.writestr("swankit-0.1.0.dist-info/licenses/release-licenses/DEPENDENCIES.json", inventory_raw)
            for name, raw in notices.items():
                out.writestr("swankit-0.1.0.dist-info/licenses/release-licenses/" + name, raw)
        self.wheel_bytes = wheel.getvalue()
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w:gz") as out:
            item = tarfile.TarInfo("swankit-0.1.0-linux-x86_64/swankit")
            item.size = 3
            out.addfile(item, io.BytesIO(b"cli"))
        values = {"swankit-0.1.0-linux-x86_64.tar.gz": archive.getvalue(),
                  "swankit-0.1.0-cp311-abi3-linux_x86_64.whl": self.wheel_bytes,
                  "LICENSE-MIT": b"license", "LICENSE-APACHE": b"license", "NOTICE": b"notice",
                  "third-party-licenses/DEPENDENCIES.json": inventory_raw,
                  **{"third-party-licenses/" + name: raw for name, raw in notices.items()},
                  "BUILD-SOURCE.json": json.dumps({"source_commit": self.commit, "source_status": [],
                       "source_files_sha256": {"Cargo.lock": self.lock},
                       "wheel_sha256": RELEASE.digest(self.wheel_bytes),
                       "dependency_licenses": {"inventory_sha256": RELEASE.digest(inventory_raw)},
                       "cli_sha256": RELEASE.digest(b"cli")}).encode()}
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w:gz") as out:
            for name, raw in {**{n: b for n, b in values.items() if not n.endswith((".tar.gz", ".whl"))}, "swankit": b"cli"}.items():
                item = tarfile.TarInfo("swankit-0.1.0-linux-x86_64/" + name)
                item.size = len(raw)
                out.addfile(item, io.BytesIO(raw))
        values["swankit-0.1.0-linux-x86_64.tar.gz"] = archive.getvalue()
        values["SHA256SUMS"] = "".join(RELEASE.digest(b) + "  " + n + "\n" for n, b in sorted(values.items())).encode()
        for name, raw in values.items():
            target = self.packet / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
        self.manifest = {"schema": "swankit-candidate/1", "version": "0.1.0", "source_commit": self.commit,
                         "lock_sha256": self.lock, "run_id": 4, "files": RELEASE.packet_files(self.packet),
                         "checks": {"ci": "PASS", "packet_smoke": "PASS"},
                         "registry": {name: {"version": RELEASE.FORMATKIT_VERSION, "source":
                              "registry+https://github.com/rust-lang/crates.io-index", "checksum": "a" * 64}
                                      for name in RELEASE.FORMATKIT}}
        self.raw = json.dumps(self.manifest).encode()
        self.receipt = {"schema": "swankit-release-approval/1", "accepted": True, "independent_review": True,
                        "tag": "v0.1.0", "source_commit": self.commit, "lock_sha256": self.lock,
                        "manifest_sha256": RELEASE.digest(self.raw), "run_id": 4, "artifact_id": 8,
                        "artifact_digest": "sha256:" + "c" * 64,
                        "corpus": {"scope": "workspace-all-features-release-corpus",
                                   "cargo_args": ["--workspace", "--all-features", "--locked", "--release", "--test", "corpus"],
                                   "exit_code": 0, "required": True, "write": False, "tests_passed": 2,
                                   "groups_passed": 1, "tests_failed": 0, "tests_ignored": 0, "tests_filtered": 0,
                                   "source_commit": self.commit,
                                   "wheel_sha256": RELEASE.digest(self.wheel_bytes)}}
        self.repo = {"private": False, "full_name": REPOSITORY, "fork": False}
        self.run = {"id": 4, "event": "workflow_dispatch", "conclusion": "success", "head_branch": "main",
                    "path": ".github/workflows/release-candidate.yml", "head_sha": self.commit,
                    "repository": {"full_name": REPOSITORY}, "head_repository": {"full_name": REPOSITORY}}
        self.artifact = {"id": 8, "expired": False, "workflow_run": {"id": 4},
                         "name": "candidate-" + self.commit, "digest": self.receipt["artifact_digest"]}

    def validate(self):
        return RELEASE.validate_promotion(self.receipt, self.raw, self.packet, self.repo, self.run,
                                          self.artifact, "v0.1.0", REPOSITORY)

    def test_valid_synthetic_approval(self):
        self.assertEqual(self.validate()["source_commit"], self.commit)

    def native_artifacts(self):
        platform = RELEASE.PLATFORM
        native = self.root / "native"
        native.mkdir()
        notices = platform.notices_from(self.packet, self.packet / "third-party-licenses")
        for target, (_, _, _, system, machines) in platform.TARGETS.items():
            directory = native / target
            directory.mkdir()
            archive, prefix, binary = platform.archive_spec("0.1.0", target)
            if system == "Windows":
                raw = b"MZ" + bytes(58) + (64).to_bytes(4, "little") + b"PE\0\0\x64\x86"
            else:
                cpu = 0x0100000C if target == "aarch64-apple-darwin" else 0x01000007
                raw = b"\xcf\xfa\xed\xfe" + cpu.to_bytes(4, "little")
            platform.write_archive(directory / archive, prefix, binary, {**notices, binary: raw})
            receipt = {
                "schema": "swankit-native-cli/1", "status": "PASS", "target": target, "version": "0.1.0",
                "source_commit": self.commit, "lock_sha256": self.lock, "archive": archive,
                "archive_sha256": RELEASE.digest((directory / archive).read_bytes()),
                "executable_sha256": RELEASE.digest(raw), "checks": sorted(platform.smoke_checks(target)),
                "tested_platform": {"system": system, "machine": machines[0], "os_version": "synthetic"},
            }
            (directory / "receipt.json").write_text(json.dumps(receipt))
        self.manifest["packet_smoke"] = {
            "status": "PASS", "cli_sha256": RELEASE.digest(b"cli"),
            "wheel_sha256": RELEASE.digest(self.wheel_bytes),
            "checks": sorted(platform.CHECKS | {"installed-wheel-core"}),
            "platform": {"system": "Linux", "machine": "x86_64", "release": "synthetic", "libc": ["glibc", "2.36"]},
        }
        (self.packet.parent / "candidate-manifest.json").write_text(json.dumps(self.manifest))
        return native

    def test_platform_candidate_requires_all_native_archives_and_updated_approval(self):
        RELEASE.assemble(self.packet.parent, self.native_artifacts(), self.commit, 4)
        self.raw = (self.packet.parent / "candidate-manifest.json").read_bytes()
        self.receipt["manifest_sha256"] = RELEASE.digest(self.raw)
        with self.assertRaisesRegex(ValueError, "approval for candidate schema"):
            self.validate()
        self.receipt["schema"] = "swankit-release-approval/2"
        manifest = self.validate()
        self.assertEqual(manifest["native_cli"].keys(), RELEASE.PLATFORM.TARGETS.keys())
        names = RELEASE.release_asset_names(self.packet, manifest)
        self.assertTrue(all(row["archive"] in names for row in manifest["native_cli"].values()))
        del manifest["native_cli"]["x86_64-pc-windows-msvc"]
        with self.assertRaisesRegex(ValueError, "incomplete native CLI matrix"):
            RELEASE.validate_packet(self.packet, manifest)

    def test_platform_assembly_rejects_different_commit_and_archive_bytes(self):
        native = self.native_artifacts()
        path = native / "x86_64-pc-windows-msvc" / "receipt.json"
        original = json.loads(path.read_text())
        path.write_text(json.dumps(dict(original, source_commit="e" * 40)))
        with self.assertRaisesRegex(ValueError, "source or smoke receipt mismatch"):
            RELEASE.assemble(self.packet.parent, native, self.commit, 4)
        path.write_text(json.dumps(original))
        (path.parent / original["archive"]).write_bytes(b"different archive")
        with self.assertRaisesRegex(ValueError, "native archive digest mismatch"):
            RELEASE.assemble(self.packet.parent, native, self.commit, 4)

    def test_refuse_changed_or_unreviewed_claims(self):
        changes = [("accepted", False), ("independent_review", False), ("manifest_sha256", "d" * 64),
                   ("source_commit", "d" * 40), ("lock_sha256", "d" * 64), ("artifact_id", 9),
                   ("artifact_digest", "sha256:" + "d" * 64), ("run_id", 5), ("tag", "v0.2.0")]
        for key, value in changes:
            before = copy.deepcopy(self.receipt)
            self.receipt[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate()
            self.receipt = before

    def test_refuse_missing_failed_empty_or_different_wheel_corpus(self):
        for key, value in [("exit_code", 1), ("required", False), ("write", True), ("tests_passed", 0),
                           ("groups_passed", 0), ("tests_failed", 1), ("wheel_sha256", "d" * 64),
                           ("source_commit", "d" * 40)]:
            before = copy.deepcopy(self.receipt)
            self.receipt["corpus"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate()
            self.receipt = before

    def test_partial_filtered_or_unidentified_corpus_scope_refused(self):
        for key, value in [("scope", "assets-only"), ("scope", None), ("tests_ignored", 1),
                           ("tests_filtered", 1), ("cargo_args", ["-p", "ws-assets", "--test", "corpus"]),
                           ("cargo_args", self.receipt["corpus"]["cargo_args"] + ["--", "one_case"])]:
            before = copy.deepcopy(self.receipt)
            self.receipt["corpus"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate()
            self.receipt = before

    def test_refuse_untrusted_failed_or_other_candidate(self):
        for key, value in [("event", "pull_request"), ("conclusion", "failure"), ("head_branch", "other"),
                           ("path", ".github/workflows/other.yml"), ("head_sha", "d" * 40),
                           ("repository", {"full_name": "other/project"}),
                           ("head_repository", {"full_name": "other/project"}), ("head_repository", None)]:
            before = copy.deepcopy(self.run)
            self.run[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate()
            self.run = before

    def test_repository_identity_is_required_regardless_of_visibility(self):
        for visibility in (False, True):
            self.repo["private"] = visibility
            self.assertEqual(self.validate()["source_commit"], self.commit)
            for changes in ({"fork": True}, {"full_name": "other/swankit"}):
                with self.subTest(visibility=visibility, changes=changes), self.assertRaisesRegex(
                        ValueError, "unexpected release repository"):
                    RELEASE.validate_repository(dict(self.repo, **changes), REPOSITORY)

    def test_refuse_expired_artifact(self):
        self.artifact["expired"] = True
        with self.assertRaisesRegex(ValueError, "expired"):
            self.validate()

    def test_changed_extra_and_symbolic_packet_files(self):
        wheel = next(self.packet.glob("*.whl"))
        wheel.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "bytes changed"):
            self.validate()
        wheel.write_bytes(self.wheel_bytes)
        (self.packet / "extra.md").write_text("unexpected")
        with self.assertRaisesRegex(ValueError, "inventory"):
            self.validate()
        (self.packet / "extra.md").unlink()
        (self.packet / "link").symlink_to(wheel)
        with self.assertRaisesRegex(ValueError, "symbolic"):
            self.validate()

    def test_archive_path_traversal_duplicate_and_symlink(self):
        for name in ("../escape", "/absolute", "part/../../escape", "part\\escape"):
            raw = io.BytesIO()
            with zipfile.ZipFile(raw, "w") as archive:
                archive.writestr(name, "synthetic")
            with self.subTest(name=name), self.assertRaises(ValueError):
                RELEASE.safe_extract_zip(raw.getvalue(), self.root / "extracted")
            self.assertFalse((self.root / "extracted").exists())

    def test_registry_rejects_path_git_and_missing_checksums(self):
        entries = [{"name": n, "version": RELEASE.FORMATKIT_VERSION, "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "checksum": "a" * 64} for n in sorted(RELEASE.FORMATKIT)]
        def lock(items):
            return "\n".join("[[package]]\n" + "\n".join(f'{k} = "{v}"' for k, v in e.items()) for e in items).encode()
        metadata = {"packages": entries}
        self.assertEqual(len(RELEASE.registry(lock(entries), metadata)), 9)
        for replacement in (None, "git+https://example.invalid/source"):
            changed = copy.deepcopy(entries)
            changed[0]["source"] = replacement
            if replacement is None:
                del changed[0]["source"]
            with self.assertRaises(ValueError):
                RELEASE.registry(lock(changed), metadata)
        changed = copy.deepcopy(entries)
        del changed[0]["checksum"]
        with self.assertRaises(ValueError):
            RELEASE.registry(lock(changed), metadata)
        previous = [{**entry, "version": RELEASE.PREVIOUS_FORMATKIT_VERSION} for entry in entries]
        with self.assertRaisesRegex(ValueError, "registry dependency version"):
            RELEASE.registry(lock(previous), {"packages": previous})

    def test_registry_must_match_version_and_checksum_of_notices(self):
        dependencies = self.manifest["registry"]
        for entry in dependencies.values():
            entry["version"] = RELEASE.PREVIOUS_FORMATKIT_VERSION
        with self.assertRaisesRegex(ValueError, "registry evidence differs from lock-bound dependency notices"):
            RELEASE.validate_packet(self.packet, self.manifest)
        dependencies["formatkit-core"]["version"] = RELEASE.FORMATKIT_VERSION
        with self.assertRaisesRegex(ValueError, "invalid registry evidence"):
            RELEASE.validate_packet(self.packet, self.manifest)
        for entry in dependencies.values():
            entry["version"] = RELEASE.FORMATKIT_VERSION
        dependencies["formatkit-core"]["checksum"] = "d" * 64
        with self.assertRaisesRegex(ValueError, "registry evidence differs from lock-bound dependency notices"):
            RELEASE.validate_packet(self.packet, self.manifest)

    def test_legacy_registry_cannot_describe_a_current_packet(self):
        old = self.manifest["registry"].pop("formatkit-patch")
        with self.assertRaisesRegex(ValueError, "invalid registry evidence"):
            RELEASE.validate_packet(self.packet, self.manifest)
        for entry in self.manifest["registry"].values():
            entry["version"] = "0.1.0"
        with self.assertRaisesRegex(ValueError, "registry evidence differs from lock-bound dependency notices"):
            RELEASE.validate_packet(self.packet, self.manifest)
        self.manifest["registry"]["formatkit-patch"] = old
        with self.assertRaisesRegex(ValueError, "invalid registry evidence"):
            RELEASE.validate_packet(self.packet, self.manifest)

    def test_real_redirect_never_forwards_authorization(self):
        observed = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                observed.append((self.path, self.headers.get("Authorization")))
                if self.path == "/initial":
                    self.send_response(302)
                    self.send_header("Location", "/storage")
                    self.end_headers()
                else:
                    self.send_response(200)
                    self.end_headers()
                    self.wfile.write(b"payload")

            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            opener = urllib.request.build_opener(RELEASE.NoCredentialRedirect(require_https=False))
            request = urllib.request.Request("http://127.0.0.1:" + str(server.server_port) + "/initial",
                                             headers={"Authorization": "Bearer synthetic"})
            self.assertEqual(opener.open(request).read(), b"payload")
            self.assertEqual(observed, [("/initial", "Bearer synthetic"), ("/storage", None)])
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_production_redirect_refuses_plain_http(self):
        handler = RELEASE.NoCredentialRedirect()
        with self.assertRaisesRegex(ValueError, "insecure"):
            handler.redirect_request(urllib.request.Request("https://example.invalid"), None, 302,
                                     "Found", {}, "http://example.invalid/file")

    def test_asset_download_and_metadata_use_distinct_accept_headers(self):
        observed = []
        class Opener:
            def open(self, request, timeout):
                observed.append((request.full_url, request.get_header("Accept")))
                return io.BytesIO(b'{}' if request.get_header("Accept") == "application/vnd.github+json" else b"asset bytes")
        api = RELEASE.GitHub(REPOSITORY, "synthetic")
        with patch.object(RELEASE.urllib.request, "build_opener", return_value=Opener()):
            self.assertEqual(api.request("GET", "/releases/assets/1", binary=True), b"asset bytes")
            self.assertEqual(api.request("GET", ""), {})
        self.assertEqual([headers for _, headers in observed],
                         ["application/octet-stream", "application/vnd.github+json"])

    def test_actions_archive_uses_json_accept_and_retains_binary_response(self):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            archive.writestr("packet/README.md", "Synthetic guide")
        raw = buffer.getvalue()
        observed = []
        class Opener:
            def open(self, request, timeout):
                observed.append(request.get_header("Accept"))
                if request.get_header("Accept") != "application/vnd.github+json":
                    raise urllib.error.HTTPError(request.full_url, 415, "Unsupported Media Type", {}, None)
                return io.BytesIO(raw)
        api = RELEASE.GitHub(REPOSITORY, "synthetic")
        destination = self.root / "downloaded"
        with patch.object(RELEASE.urllib.request, "build_opener", return_value=Opener()):
            RELEASE.download_artifact(api, 8, "sha256:" + RELEASE.digest(raw), destination)
        self.assertEqual(observed, ["application/vnd.github+json"])
        self.assertEqual((destination / "packet/README.md").read_text(), "Synthetic guide")
        self.assertEqual((self.root / "candidate.zip").read_bytes(), raw)

    def test_forbidden_payload_and_machine_path_refused(self):
        private = self.packet / ("private." + "wsc")
        private.write_bytes(bytes(16))
        with self.assertRaisesRegex(ValueError, "forbidden packet"):
            RELEASE.packet_files(self.packet)
        private.unlink()
        (self.packet / "extra.md").write_text("/" + "home" + "/" + "synthetic" + "/" + "file")
        with self.assertRaisesRegex(ValueError, "machine path"):
            RELEASE.packet_files(self.packet)

    def test_typed_wheel_markers_are_accepted_but_wheel_payload_leaks_refuse(self):
        def check(values):
            raw = io.BytesIO()
            with zipfile.ZipFile(raw, "w") as archive:
                for name, data in values.items():
                    archive.writestr(name, data)
            with zipfile.ZipFile(raw) as archive:
                RELEASE.wheel_payload(archive)
        check({"swankit/py.typed": "", "swankit/__init__.pyi": "def tiles() -> None: ...\n"})
        for suffix in ("sram", "sav", "state", "mss", "eep", "rom", "pc2", "wav", "pcm", "png"):
            with self.subTest(suffix=suffix), self.assertRaisesRegex(ValueError, "forbidden wheel"):
                check({"swankit-0.1.0.dist-info/private." + suffix: "synthetic"})
        for name in ("swankit/helper.py", "swankit/helper.pyi", "swankit-0.1.0.dist-info/METADATA"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "machine path"):
                check({name: "/" + "home" + "/" + "synthetic" + "/" + "private"})

    def test_conflicting_release_body_refuses_before_new_tag(self):
        class API:
            repository = REPOSITORY

            def release(self, tag):
                return {"body": "unrelated", "tag_name": tag}

            def request(self, method, path, body=None, binary=False):
                if method != "GET":
                    raise AssertionError("mutation before existing release conflict check")
                return {"full_name": REPOSITORY, "fork": False} if not path else None
        with self.assertRaisesRegex(ValueError, "existing release conflicts"):
            RELEASE.promote(API(), self.packet, self.raw, "v0.1.0")

    def test_existing_conflicting_tag_refuses_before_writes(self):
        class API:
            repository = REPOSITORY

            def request(self, method, path, body=None, binary=False):
                self.assert_read(method)
                if not path:
                    return {"full_name": REPOSITORY, "fork": False}
                return {"object": {"type": "commit", "sha": "d" * 40}}

            @staticmethod
            def assert_read(method):
                if method != "GET":
                    raise AssertionError("write before tag conflict check")
        with self.assertRaisesRegex(ValueError, "tag conflicts"):
            RELEASE.promote(API(), self.packet, self.raw, "v0.1.0")

    def test_zip_digest_is_checked_before_extraction(self):
        class API:
            @staticmethod
            def request(*args, **kwargs):
                return b"not the approved archive"
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            RELEASE.download_artifact(API(), 8, "sha256:" + "a" * 64, self.root / "unpacked")
        self.assertFalse((self.root / "unpacked").exists())

    def test_promotion_rerun_verifies_assets_and_refuses_conflicts(self):
        (self.root / "candidate.zip").write_bytes(b"approved transport archive")
        target = self.commit
        class API:
            repository = REPOSITORY

            def __init__(self):
                self.tag = None
                self.record = None
                self.assets = []
                self.payloads = {}
                self.writes = 0

            def release(self, tag):
                return self.record

            def request(self, method, path, body=None, binary=False):
                if method != "GET":
                    self.writes += 1
                if path.startswith("/git/ref/tags/"):
                    return self.tag
                if path.startswith("/git/tags/"):
                    return {"object": {"type": "commit", "sha": target}}
                if path == "/git/tags":
                    assert body["object"] == target
                    return {"sha": "e" * 40}
                if path == "/git/refs":
                    self.tag = {"object": {"type": "tag", "sha": body["sha"]}}
                    return self.tag
                if path == "/releases":
                    self.record = dict(body, id=1, upload_url="https://uploads.github.com/assets{?name}")
                    return self.record
                if path == "/releases/1/assets?per_page=100":
                    return self.assets
                if path.startswith("https://uploads.github.com/"):
                    from urllib.parse import parse_qs, urlsplit
                    name = parse_qs(urlsplit(path).query)["name"][0]
                    item = {"id": len(self.assets) + 1, "name": name}
                    self.assets.append(item)
                    self.payloads[item["id"]] = body
                    return json.dumps(item).encode()
                if path.startswith("/releases/assets/"):
                    return self.payloads[int(path.rsplit("/", 1)[1])]
                if path == "/releases/1" and method == "PATCH":
                    self.record.update(body)
                    return self.record
                if path == "":
                    return {"private": False, "full_name": self.repository, "fork": False}
                raise AssertionError((method, path))
        api = API()
        self.assertEqual(RELEASE.promote(api, self.packet, self.raw, "v0.1.0")["status"],
                         "PUBLISHED_AND_HASH_VERIFIED")
        writes = api.writes
        RELEASE.promote(api, self.packet, self.raw, "v0.1.0")
        self.assertEqual(api.writes, writes)
        api.record["body"] = api.record["body"].replace("Swankit ", "Private swankit ", 1)
        RELEASE.promote(api, self.packet, self.raw, "v0.1.0")
        self.assertEqual(api.writes, writes)
        api.payloads[api.assets[0]["id"]] = b"different existing asset"
        with self.assertRaisesRegex(ValueError, "existing asset conflicts"):
            RELEASE.promote(api, self.packet, self.raw, "v0.1.0")
        self.assertEqual(api.writes, writes)


if __name__ == "__main__":
    unittest.main()
