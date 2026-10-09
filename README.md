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
tools/ci.sh
```

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
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

The script creates symlinks and frozen JSON expectations from existing facts,
plus independently computed SHA-256 digests and an explicit correction ledger.
It refuses to overwrite existing staging. Source ROMs and facts are read-only.
See [ws-rom](crates/ws-rom/README.md) for the API and later crate composition.

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
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

The manifest supplies independent Java expectations, source paths and oracle
provenance. Existing expectations are preserved; a graphics spec cannot be
overwritten. No corpus population counts or golden digests live in Rust tests.

Text enrollment accepts independent string dumps and glyph image manifests:

```sh
python3 tools/stage-corpus.py --text-manifest "$TEXT_MANIFEST" --output "$STAGING_DIR"
python3 tools/stage-corpus.py --text-manifest "$EXTRA_MANIFEST" \
  --text-spec-dir ws-text-extra --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
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
