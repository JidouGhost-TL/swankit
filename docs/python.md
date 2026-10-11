# Python bindings

`swankit` is a private Python package for Python 3.11 and newer. Its PyO3
extension uses `abi3-py311`. Objects hold the existing Rust facts, definitions,
images, decoded streams, waves and PCM types. The Python layer only re-exports
these objects and provides an optional Pillow conversion.

## Container build and synthetic tests

Use the existing Rust 1.94.1 image. Python development headers, maturin, pytest
and Pillow are installed inside the container by `tools/python.Dockerfile`.
No host package or toolchain installation is needed.

```sh
SWANKIT_PYTHON_ARTIFACTS="$ARTIFACT_DIR" tools/python-container.sh
```

`tools/ci.sh` invokes this container with `--ci` or `--ci-corpus` and includes
the wheel tests in its required gates. Corpus workers import the freshly tested
wheel; host wheel installation is unnecessary.

Corpus data directories are discovered from staged symlinks and JSON paths.
If the container daemon cannot traverse private descendants on shared storage,
set `SWANKIT_CORPUS_DATA_ROOTS` to colon-separated enclosing directories that
the daemon can mount. These roots replace their discovered descendant binds
and are mounted read-only.

Choose an external artifact directory. The command builds a release wheel,
installs it inside an ephemeral container directory and runs synthetic pytest
cases against that installed wheel. It uses one Cargo job and a 4 GiB container
memory bound. Formatkit is mounted read-only from the sibling checkout or
`FORMATKIT_DIR`. If full Rust gates need formatting/lint components absent
from the base image, `SWANKIT_RUST_TOOLCHAIN_DIR` can name an existing complete
1.94.1 toolchain to mount read-only; do not install a replacement toolchain.

The wheel includes `__init__.pyi` and `py.typed`. Only the Linux x86-64 wheel is
validated here; abi3 compatibility does not establish other-platform testing.
Maturin configuration follows its [mixed package layout](https://www.maturin.rs/project_layout.html)
and PyO3's [stable ABI guidance](https://pyo3.rs/v0.27.2/building-and-distribution.html).

## Facts, detection and codecs

```python
from pathlib import Path
import swankit

rom = Path("cartridge.wsc").read_bytes()
facts = swankit.rom_facts(rom)
print(facts.size_bytes, facts.checksum_ok, facts.to_dict())
print(swankit.detect(rom, "wsc").to_dict())

packed = swankit.encode("lz-bits-16", b"sample sample")
result = swankit.decode("lz-bits-16", packed + b"padding")
assert result.data == b"sample sample"
assert result.consumed == len(packed)
```

Codecs are selected explicitly: `lz-bits-16`, `lz-rle-7f` or `okumura-lzss`.
Decoding exposes the consumed extent and accepts a bounded `max_output`.
The terminated codecs accept optional prior `history`; Okumura requires
`decoded_size` and accepts `bit_order` (`lsb-first` or `msb-first`) and `fill`.
Both input encoding and decoded output are bounded to 65536 bytes.

## Tiles, palettes and text

```python
image = swankit.tiles(bytes(16), "2bpp", columns=1)
assert (image.width, image.height, len(image.rgba)) == (8, 8, 256)
# Pillow is imported only when this helper is called.
swankit.to_pil(image).save("tile.png")

palette = swankit.color_palette(Path("palette.dat").read_bytes(), index=0)
image = swankit.tiles(bytes(32), "4bpp-planar", palette=palette, columns=1)

text = swankit.TextDefinition(Path("text-definition.json").read_text())
decoded = text.decode(b"\x01\x02\x00")
print(decoded.text, decoded.normalized, decoded.consumed)
assert text.encode_tokens(decoded) == b"\x01\x02\x00"
print(text.dump(rom, "labels"))
```

Tile formats are `2bpp`, `4bpp-planar` and `4bpp-packed`. Palettes are RGBA
bytes from `color_palette`, `mono_palette` or `grayscale`; pass exactly one
format-sized palette. Atlas options include columns, scale, cell dimensions,
column ordering and the output byte limit. Text uses the documented
[text definition format](text-definitions.md), with `builtin(id)`, canonical
`encode`, source-preserving `encode_tokens`, table `dump` and `font` atlas.
All text offsets remain relative to the supplied bytes.

## Sound, saves and IPS

```python
wave = swankit.Wave(bytes(16))
pcm = wave.to_pcm(24000)
print(pcm.samples, pcm.sample_rate, pcm.channels)
Path("preview.raw").write_bytes(pcm.s16le())

save = swankit.SaveDefinition(Path("save-definition.json").read_text())
path = Path("save-image.dat")
original = path.read_bytes()
print(save.fields, save.decode(original), save.verify(original))
changed = save.set_field(original, field=0, value=7)
path.with_name("edited-image.dat").write_bytes(changed)
assert save.verify(changed) is not None

patch = swankit.ips_create(b"before", b"after")
assert swankit.ips_apply(b"before", patch) == b"after"
```

`waves` decodes complete wave banks. `unsigned_pcm` extracts unsigned 8-bit
PCM at an explicit rate. `pcm_preview` renders a four-wave bank and ordered
`(frame, port, value)` writes to bounded 24 kHz stereo PCM; `coupled=True`
selects the owner's listening preview convention. `sound_extract` accepts
product sound definitions for waves, PCM and driver records.

Saves use [save definitions](save-definitions.md). `SaveDefinition.builtin(id)`
and `save_definitions()` expose the bundled neutral IDs. `SaveContext` accepts
the `swankit-save-context/1` JSON transport for external tables and companion
images. Pass it to save operations and `detect` when semantic candidates are
needed. `verify` returns positive validity evidence or `None`; some invalid
redundant records can coexist with a usable record. Inspect `checks_failed`
and the decoded validity fields when evaluating all records.

Edits and repairs return new `bytes`; callers choose their write destination.
`set_field` repairs transactionally by default. `repair=False` changes only
the stored field and leaves verification/repair to the caller. `fix` validates
the repaired image before returning. IPS growth and overlaps require explicit
`allow_growth`/`allow_overlap`; output limits are checked before allocation.

Rust errors retain their complete display text in `SwankitError`, a
`ValueError` subclass. Resource limits use `ResourceLimitError` and unsupported
operations use `UnsupportedError`, both subclasses of `SwankitError`.
Python argument conversion failures retain ordinary `TypeError`/`OverflowError`.

## Frozen corpus parity

`tools/stage-corpus.py --python-bindings --output "$STAGING_DIR"` creates a new
`swankit-python` namespace from the frozen product detection, natural-save and
codec specs. It copies expectations and provenance without deriving new golden
values from the binding, and refuses to replace existing specs. The Rust corpus
harness uses Formatkit `run_dir` and a persistent Python worker against the
installed wheel. The worker checks cartridge facts, detection, codecs, consumed
extents, deterministic encoding and natural-save decode/verify/repair.

Run that harness with every workspace corpus test through Formatkit's safe
runner, inside the container, with the installed wheel on `PYTHONPATH`, an
explicit `FORMATKIT_CORPUS_DIR` and `FORMATKIT_CORPUS_REQUIRED=1`. Mount the
shared lock directory at `XDG_RUNTIME_DIR` with the invoking user's UID.
Exit 75 means the lock is busy: wait and retry. Ordinary CI and pytest runs
must have all corpus variables unset. Frozen expectations are never rewritten.
