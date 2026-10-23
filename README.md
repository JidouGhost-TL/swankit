# swankit

A WonderSwan format toolkit with Rust libraries, a command-line tool and Python
bindings, built on Formatkit mechanisms. Version 0.1 reads supported formats
and provides checked save and mapped-asset editing. ROMs, save images and
per-ROM asset maps are not included.

## Quick start

The prepared distributions are a Linux x86_64 CLI archive and a Python 3.11+
abi3 wheel for Linux x86_64 with glibc 2.34 or newer. They are currently private;
crates and packages have not been published to registries. Using these
distributions does not require Rust. The [offline distribution guide](docs/distribution.md)
describes the complete packet, platform evidence and a synthetic first operation.

```sh
tar -xzf swankit-0.1.0-linux-x86_64.tar.gz
./swankit-0.1.0-linux-x86_64/swankit --version
./swankit-0.1.0-linux-x86_64/swankit --help

# Install into your chosen Python environment.
python3 -m pip install --no-index ./swankit-0.1.0-cp311-abi3-manylinux_2_34_x86_64.whl
```

The following examples use `swankit` for the extracted executable. Keep original
files and write edits to new paths. Choose a definition and field using their
documented meaning; field zero below is an example, not a universal setting.

```sh
swankit info cartridge.wsc --json
swankit save decode save-definition.json save.eeprom --json
swankit save set save-definition.json save.eeprom --field 0 --value 1 --output edited.eeprom
swankit save fix save-definition.json damaged.eeprom --output repaired.eeprom --json

swankit assets verify cartridge.wsc assets.json --json
swankit assets extract cartridge.wsc assets.json extracted
swankit assets replace cartridge.wsc assets.json --id ASSET_ID --from edited.png \
  -o edited.wsc --map-out edited-assets.json
```

Asset replacement takes the decoded representation appropriate to the mapped
asset: PNG for supported images, text for mapped strings, or decoded bytes.
Retain each output ROM with its returned map. The map preserves verified
allocation capacity across compressed edits, including undo/redo. Self-delimited
streams track consumed length; externally sized codecs such as Okumura can
retain a declared carrier length that includes an unused tail. See the
[extent and capacity rules](docs/asset-maps.md).
Growth beyond the allocation requires declared free space and complete,
expressible references.

## Supported operations and limits

| Area | Available in 0.1 | Limits |
| --- | --- | --- |
| Saves | Definition-driven decode, verification, stored-field edits and transactional repair through Rust, CLI and Python | Storage bounds and definition checks do not establish every game's menu ranges, unlock conditions or active-record selection. Some definitions require explicit context or companion images |
| Asset maps | Verification, extraction, replacement, declared relocation and reserved-capacity editing through Rust and CLI | Requires a map matching the exact ROM. Full map operations are not exposed by the Python bindings |
| Graphics, text and sound | Documented tile/palette/image, character-map/font, codec and sound operations | Supported formats and supplied definitions only; engine-specific composite formats need separate maps or implementation |
| Format search | Read-only ranked leads, bounded codec/layout probes, reference patterns and resumable parallel manifest batches | Leads require reader/source/output validation; see [format search](docs/format-search.md) |
| Discovery | Evidence-driven candidate maps with verification | Discovery is partial; captures alone do not establish original ROM storage or safe ownership |
| Patches and connections | IPS operations and the documented serial/adapter interfaces | Connection and service support have their documented interoperability limits |

A successful save edit or checksum repair does not prove that the game will
accept the value or select the edited record. Repair can update protection
values or other records beyond the selected field. Inspect decoded results,
verification flags and the complete byte diff before using an edited save.

Start with the [CLI guide](docs/cli.md), [save definitions and editing](docs/save-definitions.md),
[asset maps and replacement](docs/asset-maps.md), and [Python API](docs/python.md).
Schemas are provided for [save definitions](docs/save-definition.schema.json)
and [asset maps](docs/asset-map.schema.json). Library APIs are introduced by the
crate guides below.

## License

Most workspace code is declared `MIT OR Apache-2.0`; see [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE). Components containing the MIT-only adapter
port, and packages linking those components, declare `MIT AND (MIT OR Apache-2.0)`.
[NOTICE](NOTICE) identifies the port and preserves its attribution and license.
Distributed binaries and wheels include these texts; each crate's manifest
states its license expression.

## Libraries and development

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

[Asset discovery](docs/asset-families.md) adds `swankit assets discover` and an
adapter for ROM + runtime read/write groups + RAM/ports + decompile evidence.
Drafts are verified before writing. External family profiles define charmaps
and pointer records; new codec witnesses remain explicit sidecars until the map
codec enum supports them. Enroll frozen drafts and independent emulator outputs
with `tools/stage-corpus.py --discovery-manifest MANIFEST --output ROOT`.
