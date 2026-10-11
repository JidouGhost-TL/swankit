# Asset maps

An asset map (`swankit-asset-map/1`, JSON Schema in
[asset-map.schema.json](asset-map.schema.json)) lists the located assets of one
ROM image: where each asset is stored, how it is encoded, how it relates to
other assets, every known place that points at it, and the evidence for it.
Put the ROM and the map together and every listed asset can be extracted;
reference an asset by its id and it can be replaced.

The crate `ws-assets` reads maps; the CLI exposes it as `swankit assets`.
Grammars stay with their owners: tile, map-word and palette formats in
`ws-gfx`, codecs in `ws-compress`, character maps and fonts in `ws-text`, PCM
in `ws-sound`, IPS in `ws-patch`, footer and banking in `ws-rom`. `ws-assets`
adds placement policy only. Maps carry title labels; the crate and the CLI
are title independent, and maps live outside the repository with the corpus.

## Document

| Field | Meaning |
| --- | --- |
| `schema` | `swankit-asset-map/1` |
| `rom` | `sha256` and `size` of the one image the map describes; free `label`. Every command refuses another image |
| `charmaps` | Named character maps: `builtin` (a `ws-text` definition id), `file` (relative to the map: `swankit-charmap/1`, `swankit-text-def/1` or TSV) or `inline` |
| `palettes` | Named inline palettes (`#rrggbb` lists, 16 per hardware palette) for colours set by code immediates (monochrome shade ports) rather than ROM data |
| `free_space` | `offset`, `length`, optional uniform `fill`: the only places relocation may write |
| `assets` | The located assets (below) |
| `screens` | Composed screens: tilemap `layers` (pixel positions after scrolling; `screen` 1/2, `wrap`) and `sprites` (sprite tables, optional `sprite_window`) over a `back_color` |
| `coverage` | Free notes on what is and is not mapped |

Numbers are JSON integers or `0x` strings; maps written by the tools use
hexadecimal strings. Unknown fields are errors.

## Assets

| Field | Meaning |
| --- | --- |
| `id` | Stable id, `[A-Za-z0-9._-]`, unique in the map |
| `kind` | `tiles`, `tilemap`, `palette`, `font`, `text`, `pointer-table`, `music`, `sound-effect`, `pcm`, `data-table`, `sprite-table`, `other` |
| `offset`, `length` | File offset and stored length. `length` may be omitted for terminated codecs (the codec's end marker defines it) and for text (the charmap's end control) |
| `codec`, `codec_params` | `raw` (default) or a `ws-compress` id: `ws-lz-rle-7f`, `ws-lz-bits-16`, `ws-okumura-lzss` (with `decoded_length`, `bit_order`, `ring_fill`), `ws-lzss-u16-zero` or `ws-lzss-12` (parameters below) |
| `address` | Descriptive CPU view (`window` c0/c2/c3, `bank`, `segment`, `offset`) |
| `format` | Kind parameters (below) |
| `relations` | `tiles` (tile sets with their VRAM slot `at`, optional `skip`), `palette` / `palettes`, `charmap`, `font`, `pointer_table`, `targets` (pointer table entries) |
| `references` | Places that point at the asset (below) |
| `references_complete` | Declares that the references (plus pointer-table entries) are every pointer; required before relocation |
| `overlaps` | Ids whose byte ranges may overlap this asset (nested loads, shared string tails) |
| `confidence`, `evidence` | `high`/`medium`/`low`; `loader` (routine that reads it), `trace` (capture showing the bytes reach VRAM/RAM/sound), `notes` |

Format parameters by kind:

`ws-lzss-u16-zero` requires the original `destination` (u16 offset) and an
explicit `prehistory` policy: `low5-prefix`, `low8-prefix`, or
`zero-until-start`. Its u16 decoded size defines the stored extent when
`length` is omitted. `ws-lzss-12` requires `decoded_length` and `history`,
an array of at most 4096 byte integers immediately preceding the destination
in oldest-to-newest order. Use `[]` when there is no prior history; missing
history is refused. Its carrier size must stay unchanged on replacement.
Both codecs check exact consumption of a declared stored length. Encoders
use ordinary backward matches, so original compressed size is not guaranteed;
existing slot capacity and relocation requirements still apply.

- `tiles`: `bpp` 2 or 4, `layout` `planar`/`packed`, preview `width` (columns)
  and `palette_index` with `relations.palette`. `free_tiles` (`first`,
  `count` runs of tile indices) declares tiles the game never draws, with
  evidence; `replace --allocate` fills them. `header` `u16-count` declares a
  little-endian tile count stored before the tiles: the asset's range starts
  at the count, the decoded payload is the tiles, decoding checks the count
  and replacement rewrites it, so the set may grow or shrink (a sheet with
  any number of rows; transparent padding cells and blank cells past the old
  count are not tiles).
- `tilemap`: `width`, `height` in cells; `entry` `ws-word` (the hardware screen
  word: tile 0-8, palette 9-12, bank 13, flips 14-15; `color` false ignores the
  bank bit), `byte` (tile number plus `tile_base`, `attributes` OR-ed), or
  `index12-flip` (an engine's ROM map: 12-bit index into its own tile blob,
  flips 14-15, palette from `palette_index`). `header` `u16-width-height`
  declares little-endian width and height stored before the cells (checked
  against `width`/`height`, rewritten on replacement). `relations.tiles` places tile
  assets at VRAM slots in load order (later loads own a slot);
  `relations.palettes` places palette assets at their `palette_index`.
- `palette`: `palette_format` `rgb444` (32-byte colour RAM records) or
  `mono-lut` (2-byte port 20+2p pairs with `shades`, the eight port 1C-1F levels).
- `font`: `bpp` 1/2/4, `tiles_wide`, `tiles_high`, `order` row/column,
  `count`, `stride`, `tile_offsets`, sheet `width`; `advance` and
  `glyph_widths` for line measurement.
- `text`: `relations.charmap`; `pad` fills the unused tail of the slot.
  Optional box limits: `line_width` (pixels), `lines` (per page) and
  `page_break` (decoded spelling of the control fragment that opens a page).
  Lines end at each newline in a control's spelling; glyphs are measured with
  the `relations.font` asset's `advance` (default `tiles_wide` * 8) and its
  `glyph_widths` (pixels by decoded spelling); controls have no width.
  `replace` refuses an edit that breaks a limit; `--force` writes it and
  reports the overflow as a warning.
- `pointer-table`: `pointer` (an encoding), `stride`, `pointer_offset`, `count`
  and `relations.targets`; each listed target receives the entry as a reference.
- `pcm`: `sample_rate` (unsigned 8-bit samples, extracted as WAV).
- `sprite-table`: `entry` `ws-oam` (the hardware sprite record: attribute word
  with tile 0-8, palette 9-11 (8-15), window 12, priority 13, flips 14-15; then
  Y, X), `count` records (up to 128). `relations.tiles` and
  `relations.palettes` as for tilemaps (sprites use tile bank 0). Extracted as
  the sprites alone on a transparent 224x144 image plus the records.

Screens compose like the display (WSdev Display, Display/Sprites): back
colour, layers on screen 1 (the default), low-priority sprites, layers with
`screen: 2`, high-priority sprites. Sprite tables are concatenated in the
order listed, which is sprite-RAM order: at each pixel the first sprite with
an opaque pixel wins, and at most 32 sprites draw on one line. A screen's
`sprite_window` (ports 0C-0F, inclusive) keeps each sprite inside or outside
it per its window bit. A layer with `wrap` lies on the 256x256 screen plane:
give `x`/`y` as its map position minus the scroll registers and positions
wrap modulo 256, as hardware scrolling does.

## References

A linear reference stores
`((anchor(target) + add) >> shift) mod 2^(8 * width)` at `at` (`endian`,
`anchor` `file` or `chip`, where chip = file offset plus the padding of a
short image). Bits above the stored width are implied by the referencing code
(a bank register or a segment held elsewhere): a relocated target must keep
them, which expresses bank limits without game knowledge. Bits below `shift`
come from a companion reference (an offset beside a bank number);
`align` requires target alignment; `below` is an exclusive upper bound on the
stored value that the referencing code accepts (a loader that switches banks
only below some bank number), checked when a reference is rewritten.
`companion: true` marks a reference whose bits above `width` come from another
reference of the same asset (an offset beside its bank byte): relocation may
then change them, since the companion is rewritten too.

A far reference (`form: far`, `width` 4) is an offset16 + segment16 pair
into the 64 KiB window starting at file offset `window`, seen by the CPU at
`segment_base`. `normalize: paragraph` writes offsets below 16;
`segment` keeps the segment at `segment_base`. Targets outside the window are
not expressible. `add` designates a byte before the asset (a count word, a
map header or a marker byte) without changing the asset's own range.

## Commands

```text
swankit assets extract ROM MAP OUTDIR [--force] [--json]
swankit assets replace ROM MAP --id ID --from FILE -o PATCHED [--ips PATCH] [--map-out MAP] [--force] [--json]
swankit assets verify ROM MAP [--json]
```

`extract` writes PNG for tiles (sheet), tilemaps (rendered through their tile
and palette relations), palettes (swatch), fonts (sheet) and screens; UTF-8
`.txt` plus `.codes.tsv` (raw code units and their cell text) for text; WAV for
PCM; raw bytes for the rest and for decoded compressed graphics; and
`index.json` (offset, stored and decoded length, decoded SHA-256, files,
reference count per asset). A tilemap without a tile relation is extracted as
its words and marked `rendered: false`. Failing assets are listed in the index
and the command exits non-zero.

`replace` takes the asset's decoded form: a PNG laid out as `extract` wrote it
(tiles sheet, tilemap rendering, rgb444 swatch), a `.txt` in the charmap's
spelling, or any other file as decoded bytes. Pixels map to the cell's palette
(exact colour first, otherwise nearest; the count of nearest matches is
reported). A tilemap edit rewrites the tiles it draws and leaves the map words
alone; a tile drawn by several cells must receive identical pixels or the
command refuses. With `--allocate` the version equal to the original tile
(otherwise the most used one) keeps the slot and every other version takes a
declared free tile of a tiles asset this map loads (at the VRAM slot that
relation loads it to, and only where no later relation owns the slot); the
cells drawing it are rewritten to that slot (other entry bits kept) and the
patched map no longer lists the tile as free. Too few free tiles is refused. Text edits use `ws_text::CharacterMap::encode_edit`: every
unchanged source token is kept byte for byte (hidden control operands,
attribute bits, aliases), only the edited span is canonically encoded, and
inside that span each glyph keeps the hidden attribute bits (the bits a
`glyph_mask` ignores) of the source glyph at the same position.

Placement: the payload is re-encoded with the asset's codec. When it fits
the stored range it is written in place (the tail is filled with `pad` when
declared, otherwise left as it was; a terminated stream's declared length
becomes its new extent). When it does not fit, it is relocated only if
`references_complete` holds and every reference can express a position in a
declared free range (implied bits, alignment, far windows; a stream never newly
crosses a 64 KiB bank), first fit. The references are rewritten, the old range
is added to the output map's free space and the used free range shrinks.
Otherwise the command refuses and writes nothing. Nothing is written outside
the asset's range, a free range or a declared reference. The footer checksum is
always recomputed. `--ips` writes an IPS patch against the input; `--map-out`
writes the map updated for the patched image (identity, offsets, free space).

`verify` checks that every asset decodes and renders or parses, every range
lies in the ROM, ranges overlap only where declared, free space lies outside
assets and holds its declared fill, and every reference holds the value its
encoding gives for the asset's offset. It reports mapped bytes (union),
bytes by kind and by confidence; any error makes it exit non-zero.

## Evidence and coverage

Assets come from ROM data located through the code that loads it (loader
call sites, pointer tables) or from traces of ROM bytes reaching VRAM, RAM or
sound; never from screen captures. Mapped bytes over the ROM size measure
coverage; confidence separates loader-proven entries from trace matches.
