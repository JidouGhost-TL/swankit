# Offline distribution

Release packets include `third-party-licenses/`: the declared license metadata,
published package checksums and preserved upstream license/copyright texts for
the entire resolved Cargo package inventory. This conservative inventory also
includes build, development and target-specific packages; it does not claim
that every listed package is linked into each binary. The wheel carries the
same evidence under its distribution license directory. Generation occurs
before the wheel is built and installed for testing. Packaging refuses missing,
changed or mismatched notice files; these texts do not replace the project
licenses or change upstream license terms.

Maintainers can build and publish a reviewed private packet through the
[manual release workflows](release.md). The main `swankit` executable is
included; `wg-adapter` and `wg-decode` remain auxiliary source-build tools.

The first private release packet contains a Linux x86_64 CLI archive, a
Python 3.11+ abi3 Linux x86_64 wheel, README.md, OFFLINE.md, all linked user
and crate guides, JSON schemas, LICENSE-MIT, LICENSE-APACHE and NOTICE.
Use OFFLINE.md for the exact archive and wheel filenames. The CLI archive
contains the same guide tree; the outer tree supplies guides beside the wheel.
The wheel itself contains the Python package and legal notices. New platform
candidates also carry separate Windows x86-64, macOS Apple Silicon and macOS
Intel CLI archives with license notices. The candidate manifest records which
platform executed each archive. Consult the published release for availability.

The currently prepared artifacts require glibc 2.34 or newer according to their
ELF versioned-symbol requirements. The packet's BUILD-SOURCE.json audits the
CLI and native extension separately; a wheel tag is not evidence about the CLI.
Symbol requirements establish a lower bound, not execution on the oldest
runtime. Linux x86_64, Python 3.11 and glibc 2.36 were exercised; a wider
Python/glibc matrix, other operating systems and other architectures remain
untested. Do not install these artifacts on an incompatible platform.

Extract the CLI archive and run its executable directly. Installing the
existing wheel into your chosen environment uses no registry or Rust compiler:

```sh
python3 -m pip install --no-index --no-deps ./swankit-0.1.0-cp311-abi3-manylinux_2_34_x86_64.whl
```

The optional `swankit.to_pil` helper needs a separately supplied Pillow package.
Core tile decoding returns RGBA bytes without that dependency. No ROMs, save
images or per-ROM asset maps are included.

## A synthetic first operation

The following commands create caller-owned synthetic bytes. Set `SWANKIT` to
the extracted executable's path; use a new output directory.

```sh
python3 -c 'from pathlib import Path; Path("tile.bin").write_bytes(bytes([128, 0]) * 8)'
"$SWANKIT" tiles --help
"$SWANKIT" tiles tile.bin --format 2bpp --length 16 --columns 1 --output tile.png
python3 -c 'import swankit; image = swankit.tiles(bytes(16), "2bpp", columns=1); print(image.width, image.height, len(image.rgba))'
```

The CLI creates an 8×8 PNG with a gray left column on white; the Python example
prints `8 8 256` for a white tile. Keep originals when editing your own data.
[CLI operations](cli.md), [save definitions](save-definitions.md),
[asset maps](asset-maps.md) and [Python APIs](python.md) explain their required
inputs and limits. Successful synthetic operations do not establish game
acceptance or complete platform compatibility.

## Rebuilding the packet from existing artifacts

Maintainers run this in the pinned build container after building and checking
the CLI and wheel from the intended source checkout:

```sh
python3 tools/package-release.py --cli "$CLI" --wheel "$WHEEL" \
  --output "$NEW_PACKET_DIR" --source-commit "$FULL_SOURCE_COMMIT"
```

The output must be a new directory outside the checkout. The tool verifies
versions, Linux ELF architecture, GLIBC symbol requirements, wheel RECORD and
legal texts, and local guide links before writing. It records the full source
commit, working-tree status and source-file/artifact SHA-256 values in
BUILD-SOURCE.json. A dirty source tree is reported explicitly; hashes bind its
actual contents and do not claim a clean or reproducible build. SHA256SUMS
covers every packet file except itself. The tool assembles existing artifacts;
it does not build, download or publish them. Review the source binding and
run packet-only smoke checks before handing a packet to another user.
