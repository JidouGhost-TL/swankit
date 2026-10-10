# Frozen adapter stream enrollment

An adapter golden consists of an independently recorded serial tap and ordered
TCP request/response files. The capture's adapter supplies the response oracle;
the Rust implementation must never supply or update that expectation. Keep the
manifest, raw captures, peer bytes and frozen specs outside Git. Native failures
and old captures belong in the source exclusion ledger with explicit reasons.
These checks establish adapter compatibility, without a physical timing claim.

The manifest is a JSON object with schema
`swankit-wondergate-adapter-manifest/1`, a nonempty `samples` array, an optional
`source_excluded` array and an optional descriptive `provenance` object. Each
sample has these fields:

| Field | Meaning |
| --- | --- |
| `id` | Unique capture identity, used in external parity logs |
| `tap_path`, `tap_sha256` | Existing tap file and lowercase SHA-256 |
| `oracle` | Nonempty description of the independent implementation and capture provenance |
| `profile` | `revival` or `upstream` |
| `header` | `len16be` or `zero-then-len8`, independently selectable |
| `resolutions` | Exact hostname bytes and IPv4 addresses |
| `peers` | Ordered connections, including the BPL connection if captured |
| `expect` | Frozen recorded-byte and network digests described below |

Each resolution is `{"hostname_hex":"756e69742e696e76616c6964","ip":[127,0,0,1]}`.
Hostnames are raw nonempty bytes without a NUL, at most 255 bytes; duplicates
are rejected. No live DNS or network access occurs during replay.

Each peer declares `ip` (four integer bytes), `port` (1..65535),
`request_path`, `request_sha256`, `response_path` and `response_sha256`.
Paths identify existing independent files; hashes bind all bytes, including
NULs. Enrollment freezes the peer bytes as hexadecimal spec arguments so the
replay does not depend on a server, process timing or mutable peer paths.
The adapter's built-in BPL is disabled during corpus replay: a recorded BPL
connection must pass through the same exact request verifier as other peers.

`expect` contains `sha256` for the original tap text; `input_sha256`,
`input_length`, `output_sha256`, `output_length`; and `input_frames` and
`output_frames` arrays. Each frame has `offset`, `kind`, `cmd`, `params_length`
and `params_sha256`. Offsets refer to the joined directional wire bytes and
include skipped padding. Timestamps and read boundaries do not affect joined
streams. The tap accepts `T <u64 milliseconds> > <hex>` and `<` records plus
comments, including wrapper `T <milliseconds> # ...` metadata. Enrollment
hexadecimal bytes use lowercase, even-length strings.

The final `network` object declares `expected`, `connected`, `closed` and
`matched_requests`, each equal to the peer count, plus `mismatches: 0`.
These are success goldens: a short request, unused peer, wrong endpoint or
incomplete frame is a failure. Clients may intentionally leave response tails
unread. Every generated adapter response is compared directly to the complete
captured byte stream at input chunk sizes 1, 7 and 4096 before its digest is
accepted. Response digests alone are insufficient.

For example, assemble a manifest from independently reviewed evidence:

```python
import json
from pathlib import Path

record = json.loads(Path("independent-capture-record.json").read_text())
manifest = {
    "schema": "swankit-wondergate-adapter-manifest/1",
    "samples": [record],
    "source_excluded": [{
        "path": "old-capture.log",
        "sha256": "0" * 64,  # Replace with its independently checked hash.
        "reason": "Incomplete historical capture; no complete response oracle",
    }],
}
Path("adapter-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
```

Use absolute source paths in a real external manifest so its meaning survives
a change of working directory. Every exclusion's `path`, `sha256` and nonempty
`reason` are checked too. Excluded streams need not parse successfully and are
not copied into the curated selection. Their disposition is retained in both
`spec.json`'s `source_excluded` map and `enrollment.json` with hashes.

```sh
python3 tools/stage-corpus.py --wondergate-manifest "$ORACLE_MANIFEST" \
    --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

Enrollment independently checks the recorded tap grammar, frame offsets and
all supplied expectations. It bounds manifests and taps to 8 MiB, each peer
direction to 1 MiB, retained peer bytes across the manifest to 16 MiB, and sample, exclusion,
resolution and peer populations to 4096 each. It validates the entire manifest
before mutation, takes the shared staging lock, then atomically installs a
new `ws-wondergate-adapter` namespace containing tap symlinks, the frozen spec
and provenance sidecar. An existing namespace is always refused; other specs
are preserved. CI runs every workspace corpus handler through Formatkit's
serialized safe runner in release mode, with required mode and write mode
disabled. Ordinary synthetic tests do not load external corpus data.
