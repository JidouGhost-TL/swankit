"""Enroll independent adapter taps and TCP peers without regenerating goldens."""
import fcntl
import hashlib
import json
from pathlib import Path
import re
import tempfile

MAX_MANIFEST = 8 * 1024 * 1024
MAX_TAP = 8 * 1024 * 1024
MAX_PEER = 1024 * 1024
MAX_RETAINED_PEERS = 16 * 1024 * 1024
MAX_RECORDS = 4096


def sha(data):
    return hashlib.sha256(data).hexdigest()


def hex_bytes(value, maximum):
    if not isinstance(value, str) or len(value) > maximum * 2 or len(value) % 2 or not re.fullmatch(r"[0-9a-f]*", value):
        raise ValueError("invalid or oversized adapter hex bytes")
    return bytes.fromhex(value)


def checked_bytes(path, expected, maximum):
    if not isinstance(path, str) or not path or not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("adapter source path and SHA-256 required")
    source = Path(path).resolve(strict=True)
    if not source.is_file() or source.stat().st_size > maximum:
        raise ValueError("adapter source file exceeds bound")
    data = source.read_bytes()
    if len(data) > maximum or sha(data) != expected:
        raise ValueError("adapter bytes differ from independent oracle manifest")
    return source, data


def frames(data, header, skip_padding):
    """Independent enrollment grammar; no call to the Rust adapter or codec."""
    result = []
    offset = 0
    while offset < len(data):
        if skip_padding and data[offset] == 0:
            offset += 1
            continue
        if len(data) - offset < 4:
            raise ValueError("truncated adapter frame")
        kind, high, low, cmd = data[offset:offset + 4]
        if header == "zero-then-len8" and high:
            raise ValueError("nonzero adapter legacy length prefix")
        length = high * 256 + low
        if not 1 <= length <= 4096 or offset + length + 3 > len(data):
            raise ValueError("invalid adapter frame length")
        params = data[offset + 4:offset + length + 3]
        result.append({"offset": offset, "kind": kind, "cmd": cmd,
                       "params_length": len(params), "params_sha256": sha(params)})
        offset += length + 3
    return result


def tap_digest(data, header, skip_padding):
    """Describe recorded bytes only, never predict a response from a request."""
    streams = {">": bytearray(), "<": bytearray()}
    if len(data) > MAX_TAP:
        raise ValueError("adapter tap exceeds bound")
    try:
        lines = data.decode("ascii").splitlines()
    except UnicodeError as error:
        raise ValueError("adapter tap must be ASCII") from error
    records = 0
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) >= 3 and parts[0] == "T" and parts[2] == "#":
            if not re.fullmatch(r"[0-9]+", parts[1]) or len(parts[1]) > 20 or int(parts[1]) >= 2**64:
                raise ValueError("invalid adapter timestamp")
            continue
        if len(parts) not in (3, 4) or parts[0] != "T" or parts[2] not in streams:
            raise ValueError("invalid adapter tap record")
        if not re.fullmatch(r"[0-9]+", parts[1]) or len(parts[1]) > 20 or int(parts[1]) >= 2**64:
            raise ValueError("invalid adapter timestamp")
        streams[parts[2]].extend(hex_bytes(parts[3] if len(parts) == 4 else "", MAX_TAP // 2))
        records += 1
        if records > 1_000_000 or len(streams[parts[2]]) > MAX_TAP // 2:
            raise ValueError("adapter tap stream exceeds bound")
    if not records or not streams[">"] or not streams["<"]:
        raise ValueError("adapter golden requires both directional streams")
    result = {"sha256": sha(data)}
    for direction, label in ((">", "input"), ("<", "output")):
        stream = streams[direction]
        result.update({label + "_sha256": sha(stream), label + "_length": len(stream),
                       label + "_frames": frames(stream, header, skip_padding)})
    return result


def ip_bytes(value):
    if not isinstance(value, list) or len(value) != 4 or any(type(b) is not int or not 0 <= b <= 255 for b in value):
        raise ValueError("invalid adapter IPv4 address")
    return value


def stage_wondergate_adapter(manifest_path, output):
    repository = Path(__file__).resolve().parents[1]
    output = output.resolve()
    if output == repository or repository in output.parents:
        raise ValueError("adapter corpus must be outside the repository")
    if manifest_path.stat().st_size > MAX_MANIFEST:
        raise ValueError("oversized adapter oracle manifest")
    manifest_bytes = manifest_path.read_bytes()
    if len(manifest_bytes) > MAX_MANIFEST:
        raise ValueError("oversized adapter oracle manifest")
    manifest = json.loads(manifest_bytes)
    if not isinstance(manifest, dict) or manifest.get("schema") != "swankit-wondergate-adapter-manifest/1":
        raise ValueError("unknown adapter oracle schema")
    records = manifest.get("samples")
    exclusions = manifest.get("source_excluded", [])
    if not isinstance(records, list) or not 1 <= len(records) <= MAX_RECORDS or not isinstance(exclusions, list) or len(exclusions) > MAX_RECORDS:
        raise ValueError("bounded adapter samples and exclusions required")
    selected = []
    identities = set()
    retained_peers = 0
    for record in records:
        required = {"id", "tap_path", "tap_sha256", "oracle", "profile", "header", "resolutions", "peers", "expect"}
        if not isinstance(record, dict) or not required <= record.keys():
            raise ValueError("incomplete adapter oracle record")
        identity = record["id"]
        if not isinstance(identity, str) or not identity.strip() or identity in identities:
            raise ValueError("unique adapter sample identity required")
        identities.add(identity)
        if not isinstance(record["oracle"], str) or not record["oracle"].strip():
            raise ValueError("independent adapter provenance required")
        profile, header = record["profile"], record["header"]
        if profile not in ("upstream", "revival") or header not in ("len16be", "zero-then-len8"):
            raise ValueError("invalid adapter profile or header")
        source, tap = checked_bytes(record["tap_path"], record["tap_sha256"], MAX_TAP)
        resolutions, peers = record["resolutions"], record["peers"]
        if not isinstance(resolutions, list) or len(resolutions) > MAX_RECORDS or not isinstance(peers, list) or len(peers) > MAX_RECORDS:
            raise ValueError("bounded adapter resolutions and peers required")
        names = set()
        for resolution in resolutions:
            if not isinstance(resolution, dict):
                raise ValueError("invalid adapter resolution")
            name = hex_bytes(resolution.get("hostname_hex"), 255)
            if not name or b"\0" in name or name in names:
                raise ValueError("unique nonempty adapter hostname required")
            names.add(name)
            ip_bytes(resolution.get("ip"))
        peer_args = []
        for peer in peers:
            if not isinstance(peer, dict):
                raise ValueError("invalid adapter TCP peer")
            ip = ip_bytes(peer.get("ip"))
            port = peer.get("port")
            if type(port) is not int or not 1 <= port <= 65535:
                raise ValueError("invalid adapter TCP port")
            _, request = checked_bytes(peer.get("request_path"), peer.get("request_sha256"), MAX_PEER)
            _, response = checked_bytes(peer.get("response_path"), peer.get("response_sha256"), MAX_PEER)
            retained_peers += len(request) + len(response)
            if retained_peers > MAX_RETAINED_PEERS:
                raise ValueError("adapter peer retention exceeds bound")
            peer_args.append({"ip": ip, "port": port, "request_hex": request.hex(), "response_hex": response.hex()})
        expected = tap_digest(tap, header, profile == "revival")
        count = len(peers)
        expected["network"] = {"expected": count, "connected": count, "closed": count,
                               "matched_requests": count, "mismatches": 0}
        # JSON booleans and floating values are distinct from integer counts;
        # Python's ordinary equality would silently accept false == 0.
        if json.dumps(record["expect"], sort_keys=True) != json.dumps(expected, sort_keys=True):
            raise ValueError("frozen adapter expectation differs from recorded bytes")
        selected.append((source, {"args": {"id": identity, "profile": profile, "header": header,
                                            "resolutions": resolutions, "peers": peer_args},
                                  "expect": record["expect"]}))
    source_excluded = {}
    for exclusion in exclusions:
        if not isinstance(exclusion, dict) or not isinstance(exclusion.get("reason"), str) or not exclusion["reason"].strip():
            raise ValueError("adapter exclusion reason required")
        path, _ = checked_bytes(exclusion.get("path"), exclusion.get("sha256"), MAX_TAP)
        if str(path) in source_excluded or any(path == source for source, _ in selected):
            raise ValueError("duplicate or selected adapter exclusion")
        source_excluded[str(path)] = exclusion["reason"]
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".staging.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        directory = output / "ws-wondergate-adapter"
        if directory.exists() or directory.is_symlink():
            raise ValueError("adapter selection exists; frozen expectations can never be overwritten")
        with tempfile.TemporaryDirectory(prefix=".adapter-enroll-", dir=output) as temporary:
            staged = Path(temporary) / directory.name
            staged.mkdir()
            samples = {}
            for index, (source, sample) in enumerate(selected):
                name = f"sample-{index:06d}.tap"
                (staged / name).symlink_to(source)
                samples[name] = sample
            (staged / "spec.json").write_text(json.dumps({
                "format": directory.name, "samples": samples,
                "excluded": {"enrollment.json": "independent adapter source and exclusion provenance"},
                "source_excluded": source_excluded,
            }, indent=2, sort_keys=True) + "\n")
            (staged / "enrollment.json").write_text(json.dumps({
                "manifest_sha256": sha(manifest_bytes), "samples": records,
                "source_excluded": exclusions, "provenance": manifest.get("provenance", {}),
            }, indent=2, sort_keys=True) + "\n")
            staged.rename(directory)
    return len(selected)
