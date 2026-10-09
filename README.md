# swankit

A private Rust workspace for WonderSwan formats, built on Formatkit mechanisms.
The first crate, `ws-rom`, parses cartridge metadata, checks checksums and maps
banked ROM addresses. See [hardware rules](docs/hardware.md) and
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
