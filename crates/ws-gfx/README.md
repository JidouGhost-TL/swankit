# ws-gfx

Checked WonderSwan graphics built on Formatkit readers, fields, images and
palette application. Tile and hardware policy stays in this crate.
The `catalog` module declares owner identities for later product composition.
These headerless records require a known carrier and explicit format selection;
they do not claim content-only detection from arbitrary bytes.

`TileFormat` supports 16-byte 2bpp interleaved planar tiles and 32-byte 4bpp
planar or packed tiles. Planes increase from the low index bit; planar rows
read leftmost pixels from bit 7. Packed tiles read the high nibble first.
`decode_tile` returns 64 row-major indices; `encode_tile` is its exact
inverse (index bits above the format depth are dropped), and `render_tile` applies an RGBA
palette with checked lookup. `render_atlas` supports plain tiles, stacked
8x16 cells, row/column 16x16 cells and custom arrangements with integer scale.
Callers supply an explicit allocation limit. Incomplete tile data is an error.

`MonoPalettes` decodes eight four-bit shade levels and sixteen four-entry
shade selections from ports 1C-3F. `ColorPalettes` decodes the 512-byte RGB444
region into sixteen sixteen-entry palettes. Transparency belongs to the
display operation: index zero is transparent for palettes 4-7/12-15 in 2bpp,
and for all palettes in 4bpp. Tile previews use the supplied palette unchanged.

`Snapshot::new` borrows exactly 64 KiB RAM and 256 ports. For mono dumps,
callers explicitly zero-extend the resident RAM before constructing it.
`map` and `sprites` decode hardware records; `render_map`, `render_layer`
and `render_sprite` return `formatkit_core::Image`. Layer previews accept a
caller-selected transparent colour; sprites preserve alpha. `render` returns
the native 224x144 LCD area with opaque RGBA. It composes background, screen 1,
low sprites, screen 2 and high sprites, including wrapping scroll, flips,
inclusive windows and sprite coordinates modulo 256. Earlier sprites win.

```rust
use ws_gfx::{RenderProfile, Snapshot};

fn draw(ram: &[u8], ports: &[u8], color_hardware: bool)
    -> formatkit_core::Result<formatkit_core::Image>
{
    Snapshot::new(ram, ports, color_hardware)?.render(RenderProfile::Hardware)
}
```

`RenderProfile::Hardware` uses the documented colour 2bpp tile bank, mono
address masks, ordering across sprite priority groups and first 32 sprites per scanline. `JavaDiagnostic` reproduces
the existing extension's 2bpp bank alias, unmasked mono address bits and
priority-group ordering and unlimited sprites for independent byte parity. See [hardware notes](../../docs/hardware.md).
Both profiles use the same supplied RAM instant. This is a snapshot renderer:
it does not emulate the delayed sprite-table DMA, mid-line writes, LCD enable
timing, LCD segments or panel colour response. Optional 144 per-line port rows
support register raster effects; palette/map/tile RAM remains one snapshot.

PNG encoding belongs to the caller: all renderers return the shared Image type.
The `render_snapshot` example reads a RAM+ports concatenation and writes raw
RGBA for external comparisons or conversion. Corpus handlers are dev-only;
Java golden values and real bytes stay in an external staging root. Enroll
independent fixtures with `tools/stage-corpus.py --gfx-manifest MANIFEST
--output STAGING_ROOT`, then use the serialized required corpus gate.
Same-state emulator comparisons use `--gfx-format ws-gfx-display`. Their
expectations retain both raw RGB differences and differences after independently
measured panel response; a passing comparison gate does not imply every frame
is reconstructable from a static RAM snapshot.
