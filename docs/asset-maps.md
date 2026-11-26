# Asset maps

`ws-assets` and the `swankit` CLI accept both the universal
`formatkit.asset-map.v1` schema and the legacy `swankit-asset-map/1` schema
(JSON Schema in [asset-map.schema.json](asset-map.schema.json)). An asset map
lists the located assets of one ROM image: where each asset is stored, how it
is encoded, how it relates to other assets, every known place that points at
it, and the evidence for it.
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
| `offset`, `length` | File offset and declared stored/carrier length (see extent rules below). `length` may be omitted for terminated codecs (the codec's end marker defines it) and for text (the charmap's end control) |
| `reserved_capacity` | Optional proven total owned allocation in bytes from `offset`, independent of the consumed stream extent. Defaults to the current stored extent; retained across in-place edits and cleared on relocation |
| `codec`, `codec_params` | `raw` (default) or a `ws-compress` id: `ws-lz-rle-7f`, `ws-lz-bits-16`, `ws-lz-u8-zero`, `ws-lz-word16-msb`, `ws-lz-word-counted`, `ws-okumura-lzss` (with `decoded_length`, `bit_order`, `ring_fill`), `ws-lzss-u16-zero`, `ws-lzss-12` (parameters below), or `ws-lzs2` (decode-only) |
| `address` | Descriptive CPU view (`window` c0/c2/c3, `bank`, `segment`, `offset`) |
| `format` | Kind parameters (below) |
| `relations` | `tiles` (tile sets with their VRAM slot `at`, optional `skip`), `palette` / `palettes`, `charmap`, `font`, `pointer_table`, `targets` (pointer table entries) |
| `references` | Places that point at the asset (below) |
| `references_complete` | Declares that the references (plus pointer-table entries) are every pointer; required before relocation |
| `overlaps` | Ids whose byte ranges may overlap this asset (nested loads, shared string tails) |
| `confidence`, `evidence` | `high`/`medium`/`low`; `loader` (routine that reads it), `trace` (capture showing the bytes reach VRAM/RAM/sound), `notes` |

Format parameters by kind:

`ws-lz-u8-zero` selects the existing zero-terminated byte-command grammar.
Its stored extent includes the zero command. Omit `length` to stop at that
command; a declared length must equal the consumed extent, excluding any
following carrier bytes. Asset-map decoding uses no external history and
refuses references before the produced output. Replacement uses the existing
deterministic encoder and the map's normal allocation and reference rules.

`ws-lz-word16-msb` starts at the mode byte, excluding any carrier prefix.
Mode zero contains a u16 literal count; nonzero mode uses MSB-first flag words
and a zero match token to end the stream. Declared `length` must equal its
consumed extent. Matches require already produced bytes. The deterministic
encoder emits counted literals through 65,535 bytes, or literal flags for
65,536 bytes. It does not promise compressed size: growth needs the map's
usual capacity or relocation declarations. Extraction and codec round trips
alone do not establish compatible carrier metadata or game-side replacement;
unknown external fields stay outside the mapped allocation.

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


### LZS2 containers

Use `ws-lzs2` with the ROM offset of the four-byte signature. The decoder reads
the little-endian decoded length at header offset 4 and the token length at
offset 8. Tokens start after the 16-byte header. The map's stored extent
includes the header and the exact token span. A declared length must agree
with consumption. This codec takes its sizes from the header and supports
verification and extraction. Replacement is refused.

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

Replacement requires a ROM of at least 64 KiB (65,536 bytes). Smaller mapped
buffers can verify and extract, but cannot be replaced as cartridge images.
Asset replacements and relocated references must stay outside the ROM footer.
Replacement recomputes the sum of all bytes except the final two, modulo
65,536, and writes that little-endian checksum into those two bytes. This does not validate every
footer field or require the original checksum to be correct.

```text
swankit map verify ROM MAP [--json]
swankit map markdown MAP
swankit assets export ROM --asset-map MAP -o OUTDIR [--force] [--json]
swankit assets extract ROM MAP OUTDIR [--force] [--json]
swankit assets replace ROM MAP --id ID --from FILE -o PATCHED [--text-framing lf] [--ips PATCH] [--map-out MAP] [--force] [--json]
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

Each text asset's `.txt` contains one decoded string followed by one framing
LF byte (`0a`), including when the string is empty or already ends in CR or LF.
It is not a line-per-string table. The `.codes.tsv` companion records source
code units and their spellings; it is not replacement binary data.

Default `.txt` import keeps the legacy behavior: remove one optional final LF,
then one optional final CR, even if there was no LF. This accepts editor CRLF
endings, but can remove a content CR: payload `41 0d 00` representing `A`, CR,
terminator exports as `41 0d 0a` and default import produces `41 00` before slot
padding. Use `--text-framing lf` to remove only one optional final LF and retain
all remaining CR and LF characters. A file without a final LF is kept intact in
this mode. To encode a content LF at the end, keep the extra framing LF after
it; for content CRLF, end the file with CR LF LF. Configure the editor to
preserve these bytes. The option accepts only `lf` and applies only to `.txt`
replacement of a text asset; other values and uses are refused before writing.

For a map with a text asset named `message`, extract, edit its string while
retaining the framing LF, then reinsert it:

```sh
swankit assets extract cartridge.wsc assets.json extracted
# Edit extracted/message.txt with byte-preserving newline settings.
swankit assets replace cartridge.wsc assets.json --id message \
  --from extracted/message.txt --text-framing lf \
  -o edited.wsc --map-out edited-assets.json
swankit assets extract edited.wsc edited-assets.json checked
```

For exact decoded payload bytes, supply a file with an extension other than
`.txt` or `.png`, such as `message.bin`, without `--text-framing`. Include the
encoded character/control bytes, terminator and any required payload data;
for compressed assets these are the decoded bytes, before recompression.
Renaming a UTF-8 export does not turn it into those bytes. The binary path skips
text framing and character encoding; placement, codec and checksum rules still
apply. Text assets do not automatically export a `.bin` alongside their text.

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

For a tile sheet without a count header, transparent cells after the last tile
are padding. Replacement keeps the original tile count and refuses nontransparent
pixels in those cells. A counted tile set retains its declared growth behavior.

Placement: the payload is re-encoded with the asset's codec. When it fits
the owned allocation it is written in place (the tail is filled with `pad` when
declared, otherwise left as it was; a self-delimited stream's declared length
becomes its new consumed extent). When it does not fit, it is relocated only if
`references_complete` holds and every reference can express a position in a
declared free range (implied bits, alignment, far windows; a stream never newly
crosses a 64 KiB bank), first fit. The references are rewritten, the old range
is added to the output map's free space and the used free range shrinks.
Otherwise the command refuses and writes nothing. Apart from the final two
checksum bytes, nothing is written outside the asset's range, a free range or
a declared reference. The checksum is always recomputed. `--ips` writes an IPS
patch against the input; `--map-out` writes the map updated for the patched
image (identity, offsets, free space).

`reserved_capacity` separates placement budget from exact stream consumption.
It is a total allocation from the asset's offset, not additional bytes after
`length`. Declare extra capacity only with evidence that the asset owns the
entire allocation. It must contain the stored extent, stay inside the ROM and
outside the footer, and may not overlap declared free space. Its extra tail may
not overlap another asset or reservation, even when stored aliases have an
`overlaps` declaration. No capacity is inferred from padding or byte patterns.
Reservation checks require the other mapped assets to decode so their owned
ranges can be established.

Old maps remain valid without this field: their current decoded stored extent
is the placement budget. When a self-delimited stream shrinks, replacement
records the original extent as `reserved_capacity` in the returned map. Later
undo, redo or other replacements can use that same allocation. Explicit
reservations are retained across in-place edits. For self-delimited codecs,
`length` describes the new consumed extent. Exact-consumption checks remain in
force for codecs that require them: increasing `length` to include unused
trailing bytes is refused for those codecs.

Okumura keeps a declared carrier length on an in-place replacement. Its
`decoded_length` determines when decoding stops; consumed encoded bytes can
be shorter than the carrier. For example, a 41-byte carrier may hold a 9-byte
encoding after a smaller edit while map `length` and extraction `stored_length`
stay 41. Those fields describe the carrier, not a promise of 41 consumed bytes.
Its unchanged tail remains within the owned allocation, so restoring a larger
encoding can still fit. This is distinct from a self-delimited stream, where
replacement records consumed length and preserves the old allocation through
`reserved_capacity`. Do not infer unused space as free space. Standalone codec
`DecodeResult.consumed` measures consumed bytes; an external-length codec does
not acquire a terminator merely because the caller supplies a carrier length.

Relocation allocates only the new encoded extent from declared free space and
clears the old reservation. Spare capacity at the old offset never follows a
relocated asset. The old stored extent becomes free; an unused reserved tail is
conservatively left unallocated rather than advertised as new free space.
Always retain the returned map with its corresponding ROM. Preserve the
pristine pair as a backup and review shared tiles or overlapping assets when
applying several edits. Version 1 has no general editing-session manager.

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

Font PNG replacements use the extracted sheet geometry. Glyph row/column order,
explicit tile offsets and stride padding are preserved. Pixels use the same
nearest-palette policy as tile sheets; contradictory edits to aliased source
bytes are rejected. Partially used sheet cells remain outside the font count.

`ws-lz-word-counted` expands counted little-endian word commands with a separate literal pool. Commands select literal words, literals followed by a match, or a match in produced word history. Zero command counts and missing initial history are refused. Consumption includes the header, command table and used pool bytes; maps require the exact declared extent. Decoding and literal-only encoding must fit the current 64 KiB source window and the output is bounded to 64 KiB. The encoder may grow beyond the original allocation. This is logical software expansion; hardware transfers, runtime branch selection and game-compatible replacement are not established.

For supplied segment/base and bank parameters, [conditional table source
resolution](table-parameters.md) can produce an exact Raw candidate map before
using the existing extraction command. Its route/union and mapping conditions
remain explicit; it does not identify a named resource or change edit policy.
