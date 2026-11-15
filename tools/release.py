#!/usr/bin/env python3
"""Validate immutable candidates and promote approved bytes; never rebuild."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
import tomllib

SHA = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
PRIVATE_PAYLOAD_SUFFIXES = (".ws", ".wsc", ".pc2", ".sav", ".bin", ".eep", ".eeprom", ".rom", ".srm",
                          ".sram", ".state", ".sta", ".mss", ".wav", ".pcm", ".ogg", ".flac", ".png",
                          ".jpg", ".jpeg", ".bmp")
FORMATKIT = {"formatkit-" + name for name in
             ("core", "layout", "catalog", "runtime", "pixel", "codec", "corpus", "corpus-test")}
_LICENSE_SPEC = importlib.util.spec_from_file_location("dependency_licenses", Path(__file__).with_name("collect-dependency-licenses.py"))
DEPENDENCY_LICENSES = importlib.util.module_from_spec(_LICENSE_SPEC)
_LICENSE_SPEC.loader.exec_module(DEPENDENCY_LICENSES)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def relative(name):
    path = PurePosixPath(name)
    require(isinstance(name, str) and name and not path.is_absolute()
            and all(part not in ("", ".", "..") for part in name.split("/"))
            and "\\" not in name, "unsafe relative path")
    return path


def read_json(path):
    require(not path.is_symlink() and path.is_file(), "missing or symbolic JSON input")
    return json.loads(path.read_bytes())


def registry(lock, metadata):
    """Refuse local/git substitution of the pinned dependency family."""
    packages = {p["name"]: p for p in tomllib.loads(lock.decode())["package"]
                if p["name"].startswith("formatkit-")}
    require(FORMATKIT <= packages.keys(), "missing pinned registry dependency")
    for name, item in packages.items():
        require(item["version"] == "0.1.0" and item.get("source") ==
                "registry+https://github.com/rust-lang/crates.io-index"
                and SHA.fullmatch(item.get("checksum", "")), "non-registry dependency: " + name)
    found = {p["name"] for p in metadata["packages"] if p["name"].startswith("formatkit-")}
    require(found == packages.keys(), "resolved dependency inventory differs")
    for item in metadata["packages"]:
        if item["name"].startswith("formatkit-"):
            require(item.get("source") == packages[item["name"]]["source"]
                    and item["version"] == packages[item["name"]]["version"],
                    "resolved non-registry dependency")
    return {name: {key: item[key] for key in ("version", "source", "checksum")}
            for name, item in sorted(packages.items())}


def text_privacy(raw):
    content = raw.decode("utf-8")
    require(not re.search(r"/(?:home|mnt|media|Users|root|tmp|srv|opt)/|[A-Z]:[\\/](?:Users|Documents)[\\/]", content),
            "machine path in packet text")
    require(not re.search(r"-----BEGIN (?:OPENSSH|RSA|EC) PRIVATE KEY-----|\b(?:ghp_|sk-proj-)[A-Za-z0-9]+", content),
            "credential marker in packet text")


def wheel_payload(wheel):
    members = wheel.infolist()
    require(len({m.filename for m in members}) == len(members), "duplicate wheel member")
    for member in members:
        relative(member.filename)
        leaf = PurePosixPath(member.filename).name
        require(not member.is_dir() and ((member.external_attr >> 16) & 0o170000) != 0o120000,
                "unsafe wheel member")
        require(member.filename.endswith((".py", ".pyi", ".so")) or leaf == "py.typed"
                or ".dist-info/" in member.filename, "unexpected wheel payload")
        require(not member.filename.lower().endswith(PRIVATE_PAYLOAD_SUFFIXES), "forbidden wheel payload")
        if not member.filename.endswith(".so"):
            text_privacy(wheel.read(member))


def packet_files(packet):
    files = {}
    for path in sorted(packet.rglob("*")):
        require(not path.is_symlink(), "symbolic packet entry")
        if path.is_file():
            name = path.relative_to(packet).as_posix()
            relative(name)
            require(not name.lower().endswith(PRIVATE_PAYLOAD_SUFFIXES), "forbidden packet file")
            require(name in {"LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "SHA256SUMS"}
                    or name.startswith("third-party-licenses/")
                    or name.endswith((".md", ".json", ".py", ".tar.gz", ".whl")), "forbidden packet file")
            if not name.endswith((".tar.gz", ".whl")):
                text_privacy(path.read_bytes())
            files[name] = digest(path.read_bytes())
    return files


def validate_packet(packet, manifest):
    require(manifest.get("schema") == "swankit-candidate/1", "candidate schema")
    require(COMMIT.fullmatch(manifest.get("source_commit", "")), "source commit")
    require(re.fullmatch(r"\d+\.\d+\.\d+", manifest.get("version", "")), "version")
    require(SHA.fullmatch(manifest.get("lock_sha256", "")), "lock digest")
    files = packet_files(packet)
    require(files == manifest.get("files"), "packet inventory or bytes changed")
    require({"BUILD-SOURCE.json", "SHA256SUMS", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE"}
            <= files.keys(), "incomplete packet")
    sums = {}
    for line in (packet / "SHA256SUMS").read_text().splitlines():
        checksum, name = line.split("  ", 1)
        relative(name)
        require(SHA.fullmatch(checksum) and name not in sums, "invalid packet checksum row")
        sums[name] = checksum
    require(sums == {n: h for n, h in files.items() if n != "SHA256SUMS"}, "checksum coverage")
    source = read_json(packet / "BUILD-SOURCE.json")
    require(source["source_commit"] == manifest["source_commit"] and not source["source_status"],
            "dirty or different build source")
    require(source["source_files_sha256"]["Cargo.lock"] == manifest["lock_sha256"], "lock binding")
    dependency_bundle = packet / "third-party-licenses"
    DEPENDENCY_LICENSES.verify_bundle(dependency_bundle, manifest["lock_sha256"])
    require(source.get("dependency_licenses", {}).get("inventory_sha256") ==
            digest((dependency_bundle / "DEPENDENCIES.json").read_bytes()), "dependency inventory binding")
    wheels = [name for name in files if name.endswith(".whl")]
    archives = [name for name in files if name.endswith(".tar.gz")]
    require(len(wheels) == len(archives) == 1, "expected one wheel and CLI archive")
    require(files[wheels[0]] == source["wheel_sha256"], "wheel binding")
    with zipfile.ZipFile(packet / wheels[0]) as wheel:
        wheel_payload(wheel)
        info = next(n for n in wheel.namelist() if n.endswith(".dist-info/RECORD"))
        prefix = info.rsplit("/", 1)[0] + "/licenses/release-licenses/"
        included = {n[len(prefix):]: wheel.read(n) for n in wheel.namelist() if n.startswith(prefix)}
        expected = {p.relative_to(dependency_bundle).as_posix(): p.read_bytes()
                    for p in dependency_bundle.rglob("*") if p.is_file()}
        require(included == expected, "wheel dependency notices differ or are missing")
    require(FORMATKIT <= manifest.get("registry", {}).keys(), "missing registry evidence")
    for item in manifest["registry"].values():
        require(item.get("version") == "0.1.0" and item.get("source") ==
                "registry+https://github.com/rust-lang/crates.io-index"
                and SHA.fullmatch(item.get("checksum", "")), "invalid registry evidence")
    guides = {name: checksum for name, checksum in files.items()
              if name not in {wheels[0], archives[0], "SHA256SUMS"}}
    prefix = "swankit-" + manifest["version"] + "-linux-x86_64/"
    with tarfile.open(packet / archives[0], "r:gz") as archive:
        names = set()
        for member in archive:
            relative(member.name)
            require(member.isfile() and member.name not in names, "unsafe or duplicate CLI archive member")
            require(member.name.startswith(prefix), "unexpected archive prefix")
            inside = member.name[len(prefix):]
            require(inside == "swankit" or inside in guides, "unexpected archive payload")
            if inside != "swankit":
                require(digest(archive.extractfile(member).read()) == guides[inside], "archive guide differs")
            names.add(member.name)
        executable = [n for n in names if n.endswith("/swankit")]
        require(len(executable) == 1, "missing CLI")
        require(names == {prefix + name for name in guides} | {prefix + "swankit"}, "incomplete archive guides")
        require(digest(archive.extractfile(executable[0]).read()) == source["cli_sha256"], "CLI binding")
    return {"wheel": wheels[0], "archive": archives[0], "files": files}


def validate_promotion(receipt, manifest_raw, packet, repo, run, artifact, tag, repository):
    require(repo.get("private") is True and repo.get("full_name") == repository, "repository must be private")
    require(receipt.get("schema") == "swankit-release-approval/1"
            and receipt.get("accepted") is True and receipt.get("independent_review") is True,
            "missing reviewed approval")
    manifest = json.loads(manifest_raw)
    contents = validate_packet(packet, manifest)
    require(tag == "v" + manifest["version"] and receipt.get("tag") == tag, "tag/version mismatch")
    require(receipt.get("manifest_sha256") == digest(manifest_raw), "manifest approval mismatch")
    require(receipt.get("source_commit") == manifest["source_commit"]
            and receipt.get("lock_sha256") == manifest["lock_sha256"], "source approval mismatch")
    require(run.get("event") == "workflow_dispatch" and run.get("conclusion") == "success"
            and run.get("head_branch") == "main" and run.get("path") == ".github/workflows/release-candidate.yml"
            and run.get("head_repository", {}).get("full_name") == repository,
            "not a successful trusted candidate run")
    require(run.get("head_sha") == manifest["source_commit"], "candidate run/source mismatch")
    require(receipt.get("run_id") == run.get("id") == manifest.get("run_id"), "run approval mismatch")
    require(receipt.get("artifact_id") == artifact.get("id") and artifact.get("expired") is False,
            "artifact missing, expired or unapproved")
    require(artifact.get("workflow_run", {}).get("id") == run["id"], "artifact run mismatch")
    require(artifact.get("name") == "candidate-" + manifest["source_commit"], "artifact name mismatch")
    require(artifact.get("digest") == receipt.get("artifact_digest")
            and re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("digest", "")), "artifact digest approval mismatch")
    corpus = receipt.get("corpus", {})
    expected_args = ["--workspace", "--all-features", "--locked", "--release", "--test", "corpus"]
    require(corpus.get("scope") == "workspace-all-features-release-corpus"
            and [a for a in corpus.get("cargo_args", []) if a != "--offline"] == expected_args,
            "corpus scope is not the complete required workspace gate")
    require(type(corpus.get("exit_code")) is int and corpus["exit_code"] == 0 and corpus.get("required") is True
            and corpus.get("write") is False
            and type(corpus.get("tests_passed")) is int and corpus["tests_passed"] > 0
            and type(corpus.get("groups_passed")) is int and corpus["groups_passed"] > 0
            and all(type(corpus.get(key)) is int and corpus[key] == 0
                    for key in ("tests_failed", "tests_ignored", "tests_filtered"))
            and corpus.get("wheel_sha256") == contents["files"][contents["wheel"]]
            and corpus.get("source_commit") == manifest["source_commit"], "corpus receipt not qualified")
    require(manifest.get("checks") == {"ci": "PASS", "packet_smoke": "PASS"}, "candidate checks missing")
    return manifest


class NoCredentialRedirect(urllib.request.HTTPRedirectHandler):
    """Object storage redirects never receive the repository authorization."""
    def __init__(self, require_https=True):
        self.require_https = require_https

    def redirect_request(self, request, response, code, message, headers, newurl):
        require(not self.require_https or urllib.parse.urlsplit(newurl).scheme == "https",
                "insecure download redirect")
        redirected = super().redirect_request(request, response, code, message, headers, newurl)
        if redirected is not None:
            redirected.remove_header("Authorization")
        return redirected


class GitHub:
    def __init__(self, repository, token):
        require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository), "invalid repository")
        self.root = "https://api.github.com/repos/" + repository
        self.token = token

    def request(self, method, path, body=None, binary=False):
        url = path if path.startswith("https://") else self.root + path
        require(urllib.parse.urlsplit(url).hostname in {"api.github.com", "uploads.github.com"}, "untrusted API URL")
        headers = {"Authorization": "Bearer " + self.token,
                   "Accept": "application/octet-stream" if binary and method == "GET" else "application/vnd.github+json",
                   "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "swankit-release"}
        if body is not None:
            headers["Content-Type"] = "application/octet-stream" if binary else "application/json"
            body = body if binary else json.dumps(body).encode()
        try:
            opener = urllib.request.build_opener(NoCredentialRedirect())
            with opener.open(urllib.request.Request(url, data=body, headers=headers, method=method), timeout=120) as response:
                raw = response.read()
                return raw if binary else json.loads(raw)
        except urllib.error.HTTPError as error:
            if error.code == 404 and method == "GET":
                return None
            raise

    def release(self, tag):
        return self.request("GET", "/releases/tags/" + urllib.parse.quote(tag, safe=""))


def safe_extract_zip(raw, destination):
    import io
    require(not destination.exists(), "artifact destination exists")
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        names = set()
        total = 0
        for member in archive.infolist():
            relative(member.filename)
            require(member.filename == "candidate-manifest.json" or member.filename.startswith("packet/"),
                    "unexpected artifact payload")
            require(not member.is_dir() and member.filename not in names, "duplicate or directory artifact member")
            require(((member.external_attr >> 16) & 0o170000) != 0o120000, "symbolic artifact member")
            total += member.file_size
            require(total <= 512 * 1024 * 1024, "artifact expansion too large")
            names.add(member.filename)
        destination.mkdir(parents=True)
        for member in archive.infolist():
            path = destination / member.filename
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as output:
                output.write(archive.read(member))


def download_artifact(api, artifact_id, expected_digest, destination):
    # NoCredentialRedirect removes authorization before every download redirect.
    raw = api.request("GET", "/actions/artifacts/" + str(artifact_id) + "/zip", binary=True)
    require(raw is not None and "sha256:" + digest(raw) == expected_digest, "downloaded artifact digest mismatch")
    safe_extract_zip(raw, destination)
    (destination.parent / "candidate.zip").write_bytes(raw)


def promote(api, packet, manifest_raw, tag):
    manifest = json.loads(manifest_raw)
    target = manifest["source_commit"]
    tag_path = "/git/ref/tags/" + urllib.parse.quote(tag, safe="")
    existing = api.request("GET", tag_path)
    if existing:
        obj = existing["object"]
        if obj["type"] == "tag":
            obj = api.request("GET", "/git/tags/" + obj["sha"])["object"]
        require(obj["type"] == "commit" and obj["sha"] == target, "existing tag conflicts")
    marker = "Approved candidate manifest SHA-256: " + digest(manifest_raw)
    release = api.release(tag)
    body = "Private swankit " + manifest["version"] + "\n\n" + marker + "\n\n" + \
        "Linux x86_64 CLI and Python abi3 wheel; see the offline guides for tested platforms and limits."
    if release:
        require(release["body"] == body and release["tag_name"] == tag, "existing release conflicts")
    expected = dict(manifest["files"])
    expected["candidate-manifest.json"] = digest(manifest_raw)
    bundle = packet.parent.parent / "candidate.zip"
    expected["swankit-" + manifest["version"] + "-packet.zip"] = digest(bundle.read_bytes())
    assets = api.request("GET", "/releases/" + str(release["id"]) + "/assets?per_page=100") if release else []
    require(len(assets) < 100, "unexpected release asset pagination")
    by_name = {asset["name"]: asset for asset in assets}
    # Ship one packet archive, wheel and manifest; docs are inside the archive.
    names = {n for n in expected if n.endswith((".tar.gz", ".whl"))}
    names |= {"candidate-manifest.json", "SHA256SUMS", "BUILD-SOURCE.json",
              "swankit-" + manifest["version"] + "-packet.zip"}
    require(by_name.keys() <= names, "unexpected existing release asset")
    for name, asset in by_name.items():
        raw = api.request("GET", "/releases/assets/" + str(asset["id"]), binary=True)
        require(digest(raw) == expected[name], "existing asset conflicts")
    require(release is None or release["draft"] or by_name.keys() == names, "published release is incomplete")
    # All known conflicts are checked before the first external mutation.
    if not existing:
        annotation = api.request("POST", "/git/tags", {"tag": tag, "message": "swankit " + tag,
                                 "object": target, "type": "commit",
                                 "tagger": {"name": "jidoughost", "email": "hello@jidoughost.com"}})
        api.request("POST", "/git/refs", {"ref": "refs/tags/" + tag, "sha": annotation["sha"]})
    if release is None:
        release = api.request("POST", "/releases", {"tag_name": tag, "target_commitish": target,
                              "name": tag, "body": body, "draft": True, "prerelease": False})
    for name in sorted(names):
        if name in by_name:
            continue
        else:
            require(release["draft"] is True, "published release is incomplete")
            raw = (manifest_raw if name == "candidate-manifest.json" else bundle.read_bytes()
                   if name.endswith("-packet.zip") else (packet / name).read_bytes())
            uploaded = api.request("POST", release["upload_url"].split("{", 1)[0] + "?name=" + urllib.parse.quote(name), raw, binary=True)
            # Binary upload returns JSON, despite its request content type.
            asset = json.loads(uploaded)
            downloaded = api.request("GET", "/releases/assets/" + str(asset["id"]), binary=True)
            require(digest(downloaded) == expected[name], "uploaded asset digest mismatch")
    if release["draft"]:
        api.request("PATCH", "/releases/" + str(release["id"]), {"draft": False})
    require(api.request("GET", "")["private"] is True, "repository visibility changed")
    return {"status": "PUBLISHED_AND_HASH_VERIFIED", "tag": tag, "source_commit": target,
            "release_id": release["id"], "manifest_sha256": digest(manifest_raw)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    reg = sub.add_parser("registry")
    reg.add_argument("--lock", type=Path, required=True)
    reg.add_argument("--metadata", type=Path, required=True)
    reg.add_argument("--output", type=Path, required=True)
    make = sub.add_parser("manifest")
    make.add_argument("--packet", type=Path, required=True)
    make.add_argument("--source", type=Path, required=True)
    make.add_argument("--run-id", type=int, required=True)
    make.add_argument("--registry", type=Path, required=True)
    make.add_argument("--smoke", type=Path, required=True)
    make.add_argument("--environment", type=Path, required=True)
    make.add_argument("--output", type=Path, required=True)
    pub = sub.add_parser("promote")
    pub.add_argument("--receipt", type=Path, required=True)
    pub.add_argument("--repository", required=True)
    pub.add_argument("--run-id", type=int, required=True)
    pub.add_argument("--artifact-id", type=int, required=True)
    pub.add_argument("--tag", required=True)
    pub.add_argument("--destination", type=Path, required=True)
    pub.add_argument("--output", type=Path, required=True)
    pub.add_argument("--publish", action="store_true")
    final = sub.add_parser("download-release")
    final.add_argument("--receipt", type=Path, required=True)
    final.add_argument("--repository", required=True)
    final.add_argument("--tag", required=True)
    final.add_argument("--destination", type=Path, required=True)
    final.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.mode == "registry":
        result = registry(args.lock.read_bytes(), read_json(args.metadata))
    elif args.mode == "manifest":
        require(args.run_id > 0, "run id")
        require(read_json(args.smoke).get("status") == "PASS", "packet smoke not passed")
        dependencies = read_json(args.registry)
        require(FORMATKIT <= dependencies.keys(), "registry evidence missing")
        result = {"schema": "swankit-candidate/1", "source_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=args.source, text=True).strip(),
            "version": tomllib.loads((args.source / "Cargo.toml").read_text())["workspace"]["package"]["version"],
            "lock_sha256": digest((args.source / "Cargo.lock").read_bytes()), "run_id": args.run_id,
            "files": packet_files(args.packet), "checks": {"ci": "PASS", "packet_smoke": "PASS"},
            "registry": dependencies, "packet_smoke_sha256": digest(args.smoke.read_bytes()),
            "build_environment": read_json(args.environment), "packet_smoke": read_json(args.smoke)}
        source_binding = read_json(args.packet / "BUILD-SOURCE.json")
        smoke_receipt = read_json(args.smoke)
        require(smoke_receipt.get("cli_sha256") == source_binding["cli_sha256"]
                and smoke_receipt.get("wheel_sha256") == source_binding["wheel_sha256"], "smoke artifact mismatch")
        validate_packet(args.packet, result)
    elif args.mode == "download-release":
        receipt = read_json(args.receipt)
        require(receipt.get("tag") == args.tag and re.fullmatch(r"v\d+\.\d+\.\d+", args.tag), "release tag mismatch")
        api = GitHub(args.repository, os.environ["GH_TOKEN"])
        require(api.request("GET", "")["private"] is True, "repository must remain private")
        release = api.release(args.tag)
        require(release is not None and release["draft"] is False, "published release missing")
        assets = api.request("GET", "/releases/" + str(release["id"]) + "/assets?per_page=100")
        bundle_name = "swankit-" + args.tag[1:] + "-packet.zip"
        bundle = next((a for a in assets if a["name"] == bundle_name), None)
        require(bundle is not None, "release packet missing")
        raw = api.request("GET", "/releases/assets/" + str(bundle["id"]), binary=True)
        require("sha256:" + digest(raw) == receipt.get("artifact_digest"), "published packet digest mismatch")
        safe_extract_zip(raw, args.destination)
        manifest_raw = (args.destination / "candidate-manifest.json").read_bytes()
        manifest = json.loads(manifest_raw)
        require(digest(manifest_raw) == receipt.get("manifest_sha256")
                and manifest["source_commit"] == receipt.get("source_commit"), "published manifest mismatch")
        validate_packet(args.destination / "packet", manifest)
        tag_ref = api.request("GET", "/git/ref/tags/" + urllib.parse.quote(args.tag, safe=""))["object"]
        if tag_ref["type"] == "tag":
            tag_ref = api.request("GET", "/git/tags/" + tag_ref["sha"])["object"]
        require(tag_ref["type"] == "commit" and tag_ref["sha"] == manifest["source_commit"], "published tag mismatch")
        result = {"status": "DOWNLOADED_AND_HASH_VERIFIED", "source_commit": manifest["source_commit"]}
    else:
        require(args.run_id > 0 and args.artifact_id > 0, "positive run/artifact identifiers required")
        require(re.fullmatch(r"v\d+\.\d+\.\d+", args.tag), "invalid release tag")
        receipt = read_json(args.receipt)
        api = GitHub(args.repository, os.environ["GH_TOKEN"])
        repo = api.request("GET", "")
        run = api.request("GET", "/actions/runs/" + str(args.run_id))
        artifact = api.request("GET", "/actions/artifacts/" + str(args.artifact_id))
        require(run is not None and artifact is not None, "candidate missing")
        require(receipt.get("artifact_id") == args.artifact_id and receipt.get("artifact_digest") == artifact.get("digest"),
                "artifact approval mismatch")
        download_artifact(api, args.artifact_id, artifact["digest"], args.destination)
        manifest_raw = (args.destination / "candidate-manifest.json").read_bytes()
        packet = args.destination / "packet"
        validate_promotion(receipt, manifest_raw, packet, repo, run, artifact, args.tag, args.repository)
        result = promote(api, packet, manifest_raw, args.tag) if args.publish else {"status": "VALIDATED_NOT_PUBLISHED"}
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
