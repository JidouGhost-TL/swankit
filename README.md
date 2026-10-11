# swankit

A private Rust workspace for WonderSwan formats, built on Formatkit mechanisms.
`ws-rom` parses cartridge metadata, checks checksums and maps banked ROM
addresses. [ws-compress](crates/ws-compress/README.md) provides bounded
compression streams with deterministic encoders. [ws-gfx](crates/ws-gfx/README.md)
decodes tiles, palettes, maps and sprites and renders display snapshots to RGBA
images. [ws-text](crates/ws-text/README.md) provides data-defined character
maps, checked text tables, encoding and glyph extraction; see
[text definitions](docs/text-definitions.md) for read/write examples.
See [hardware rules](docs/hardware.md) and
[contributor rules](CONTRIBUTING.md).

Requires the existing Rust 1.94.1 toolchain with rustfmt and Clippy.

```sh
SWANKIT_PYTHON_ARTIFACTS="$ARTIFACT_DIR" tools/ci.sh
```

CI delegates host invocations to the pinned Python container. It includes
no-default-features compilation, the CLI without saves, and installed-wheel
tests; the same wheel is used for Python corpus checks. Artifact directories
stay external, and staged symlinks/reference files are mounted read-only.
Interop bootstrap mode is forbidden during CI and required corpus validation.

Formatkit packages are requested from crates.io at exactly `=0.1.0`. During
development, `[patch.crates-io]` redirects them to the sibling `../formatkit`
checkout. Relative paths keep machine locations out of Git. To switch off the
override, remove the entire patch table, run `cargo update`, review and commit
the changed lockfile, then run the gates. Publication remains disabled.
The safe corpus runner is supplied separately by the sibling checkout or
`FORMATKIT_DIR`; the libraries remain pinned registry dependencies.

Corpus staging and golden specs stay outside this repository. Rebuild a fresh
staging root with:

```sh
python3 tools/stage-corpus.py --titles "$TITLES_DIR" --output "$STAGING_DIR"
SWANKIT_PYTHON_ARTIFACTS="$ARTIFACT_DIR" FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

The script creates symlinks and frozen JSON expectations from existing facts,
plus independently computed SHA-256 digests and an explicit correction ledger.
It refuses to overwrite existing staging. Source ROMs and facts are read-only.
See [ws-rom](crates/ws-rom/README.md) for the API and later crate composition.

Enroll independent adapter serial captures and ordered TCP peers with
`tools/stage-corpus.py --wondergate-manifest "$ORACLE_MANIFEST" --output "$STAGING_DIR"`.
The new `ws-wondergate-adapter` selection verifies full response bytes at three
input chunk sizes, exact peer requests and connection closure. Old and failed
captures carry explicit exclusion reasons. See the
[adapter manifest guide](docs/wondergate-adapter-manifest.md) for its schema,
limits and frozen provenance. `tools/ci.sh --corpus` runs required corpus
checks in release mode through the serialized safe runner.

Add a new codec namespace from an independently produced manifest with
`tools/stage-corpus.py --codec-manifest "$ORACLE_MANIFEST" --output "$STAGING_DIR"`.
The manifest supplies raw/stored/reference-encoded paths, independently checked
hashes, consumed extents, evidence and slot-fit policy. Staging refuses to
overwrite an existing codec namespace or pair set. It writes Formatkit codec
pairs and frozen JSON digests outside Git. Run parity with the same explicit
root and safe runner as cartridge validation; see the codec crate's README.

Graphics enrollment adds a new `ws-gfx` spec directory to an external root:

```sh
python3 tools/stage-corpus.py --gfx-manifest "$GFX_MANIFEST" --output "$STAGING_DIR"
SWANKIT_PYTHON_ARTIFACTS="$ARTIFACT_DIR" FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

The manifest supplies independent Java expectations, source paths and oracle
provenance. Existing expectations are preserved; a graphics spec cannot be
overwritten. No corpus population counts or golden digests live in Rust tests.

Text enrollment accepts independent string dumps and glyph image manifests:

```sh
python3 tools/stage-corpus.py --text-manifest "$TEXT_MANIFEST" --output "$STAGING_DIR"
python3 tools/stage-corpus.py --text-manifest "$EXTRA_MANIFEST" \
  --text-spec-dir ws-text-extra --output "$STAGING_DIR"
SWANKIT_PYTHON_ARTIFACTS="$ARTIFACT_DIR" FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

Both new text selections preserve existing frozen specs and version ledgers.
The extra manifest covers pointer-table references and partial-dump diagnostics.
Source and oracle hashes are checked before enrollment; all text and image
expectations remain external.

[ws-sound](crates/ws-sound/README.md) adds wave RAM decoding, sound register and
noise policy, decoded driver records and bounded 24 kHz PCM previews. Independent
sound expectations are enrolled from an external manifest:

```sh
python3 tools/stage-corpus.py --sound-manifest "$SOUND_MANIFEST" --output "$STAGING_DIR"
```

This creates a new `ws-sound` spec directory without changing existing specs.
The additional `ws-sound-extra` and `ws-sound-captures` selections use
`--sound-spec-dir`. All three remain external and are checked by the safe runner.

[ws-patch](crates/ws-patch/README.md) parses, validates, applies and creates IPS
patches, including RLE records and the optional EOF size extension. Overlap,
growth and output bounds are explicit application policy. Enroll independent
patch/base/target evidence with `--patch-manifest` in `tools/stage-corpus.py`;
the new frozen `ws-patch` selection is checked by the same safe corpus runner.

[ws-save](crates/ws-save/README.md) provides save media, internal EEPROM owner
profiles and parameterized checksums. [ws-saves](crates/ws-saves/README.md)
interprets per-game JSON definitions for decoding, stored-field edits,
verification and transactional repair. See the versioned
[save definition format and read/write guide](docs/save-definitions.md).

[swankit](crates/swankit/README.md) composes the owner declarations into a
validated Formatkit runtime catalog and typed product APIs. The
[command line](docs/cli.md) provides `info`, `detect`, PNG tiles and snapshots,
definition-driven text/font/sound/codec extraction, save decode/set/fix and IPS
create/apply:

```sh
cargo build -p swankit-cli --locked
cargo run -p swankit-cli --locked -- info cartridge.wsc --json
```

Enroll product detection from the existing frozen ROM and IPS selections with
`--product-detection` in the staging tool. Expectations and provenance remain
external; existing selections cannot be overwritten. `--product-saves` adds
a separate selection for natural-save detection, decoding and independently
recorded repair bytes and trees.

[Python bindings](docs/python.md) expose the Rust facts, detection, RGBA tiles
and palettes, codecs and consumed extents, text definitions, sound previews,
transactional saves and IPS operations as the private `swankit` package.
`tools/python-container.sh` builds and tests its Python 3.11+ abi3 wheel inside
a container, keeping wheel artifacts external. `--python-bindings` in the
staging tool enrolls a new parity selection from existing frozen owner specs.

[swankit-ffi](crates/swankit-ffi/README.md) is the experimental Java interop
boundary for cartridge facts and 4bpp RGBA decoding. It exposes a versioned
C ABI and bounded CLI JSON-lines requests; format logic stays in the owner
crates. Independent Java vectors are enrolled with `--interop-manifest`.

[ws-wondergate](crates/ws-wondergate/README.md) models the WonderGate adapter as
an injected-network request/reply core, with streaming framing, upstream and
revival profiles, bounded commands, TCP with explicit DNS policy and timeouts,
Scripted and checked transcript replay, a local 24-byte BPL responder, redirects
and INI configuration. See [protocol and configuration](docs/wondergate.md).
`wg-adapter` is the raw Mednafen excomm executable (including its `ASDF`
argument, environment configuration, EOF exit and optional lossless tap).
`wg-decode` lists recorded frames; both also run as `swankit` subcommands.
Recorded adapter corpus enrollment follows in the next milestone.

[ws-link](crates/ws-link/README.md) and `swankit link` join serial byte streams
through TCP, stdio, shell helpers, raw devices or the existing WonderGate core.
See [link recipes and interoperability limits](docs/link.md).

[ws-wondergate-server](crates/ws-wondergate-server/README.md) provides bounded,
byte-preserving HTTP and shared BPL/charge services, injected clock and entropy,
empty memory histories, a durable JSON-lines journal and scoped ranking boards.
`swankit wg-server --data DIR --wire DIR` starts threaded loopback TCP listeners
and writes lossless Python-compatible wire logs. Service identities live in
JSON data. The bounded definition engine serves bundled phase, check-in, ranking,
stock and conservative content-refusal routes. See the [definition read/write
guide and plugin ledger](docs/wondergate-title-definitions.md). Captured title
HTTP flows enroll separately in `ws-wondergate-server`; the in-process adapter
remains a later milestone.
