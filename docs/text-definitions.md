# Text definitions

`ws-text` interprets metadata selected by the caller. A byte sequence has no
universal text identity: select the encoding and source revision before reading
or writing it. All offsets are zero-based byte offsets in the supplied source;
JSON numbers are decimal. Definition IDs are neutral. Title associations and
source hashes belong in an external corpus manifest.

## Definition structure

`swankit-text-def/1` contains `id`, `map`, named `fonts` and named `tables`.
The latter two objects default to empty. `TextDefinition::from_json` rejects
unknown fields/schema versions and validates its character map. Font and table
operations validate their extents against the actual source when used.
JSON definitions are limited to 4 MiB. The nested map uses
`schema: "swankit-charmap/1"`:

| Field | Meaning |
| --- | --- |
| `encoding` | `table` (default), `shift-jis` or `jis-x0201` |
| `unit` | `byte` (default), `word-le` or `word-be`; standard encodings require `byte` |
| `initial_page` | Page at string entry, default zero |
| `entries` | `bytes` in hexadecimal **source order**, nonempty `text`, optional `page` and readable `aliases` |
| `controls` | Code/mask matches, operand lengths, display forms, termination and page effects |
| `glyph_mask` | Optional `below` threshold and bit `mask`; applied to glyph lookup after control matching |
| `normalization` | Optional kana composition, long dash after kana, or contextual dash rule |

An entry may contain multiple code units. Decoding selects the longest matching
sequence on the current page. A missing `page` makes an entry global; global
and scoped entries with identical bytes cannot overlap. Controls have priority,
so entries whose first unit overlaps a control are rejected. Duplicate entries,
overlapping controls, invalid ranges and partial units are errors. Readable
aliases may be ambiguous; canonical encoding uses the first definition entry
among equally long spellings and checks that decoding reproduces the text.

TSV maps are a compact alternative. Columns are `bytes`, `text` as a JSON
string, and optional decimal `page`. Empty trailing page columns are accepted.
Blank lines and lines starting `#` are ignored. TSV cannot declare controls;
use JSON for a complete interpreter grammar.

```text
bytes   text    page
01      "A"
02      "\n"    0
```

Use actual tabs between columns, not the alignment spaces shown above.

## Controls and normalization

Each control matches `(unit & mask) == code`. The default mask covers the
entire unit. `argument_bytes` defaults to zero. Nonoverlapping
`argument_lengths` rows (`low_min`, `low_max`, `bytes`) override it for the
low byte of the matched unit. `argument_unit` specifies operand byte order
and defaults to `byte`; lengths must contain whole operand units. Per-control
operand bytes are limited to 256. `set_page` changes the active page and
`ends` stops decoding after that control and its operands.

`display` is literal Unicode text, with optional `{lo:02x}`, `{lo:d}` and
`{args}` placeholders. Operand values use zero-padded hexadecimal preceded by
`argument_separator`. Nonoverlapping `display_overrides` rows choose another
template by low-byte range. A single `{args}` placeholder is supported.
Controls may have an empty display, including invisible switches and ends.
Canonical encoding represents hidden operands as zero; exact operand values
always require source tokens.

Compiled maps bound retained entry spellings to 4 MiB and expanded control
spellings to 8 MiB, including when built directly from Rust structs. These
limits are checked before retaining another entry or control spelling.

`DecodedText.text` follows the oracle's raw cell conventions. Contextual dash
resolution is applied to that text; tokens retain the original cell spelling
and bytes. `normalized` is a readable view: spacing dakuten/handakuten become
combining marks and compose with NFC when `compose_kana` is set. The optional
`long_dash_after_kana` rule changes a full-width minus after kana to a long
vowel mark. A contextual dash rule supplies a placeholder character and the
counter characters that prefer the kanji reading. These rules are encoding
policy, not WonderSwan hardware behavior.

Encode `text`, rather than `normalized`, when the map encodes separate marks.
`encode_tokens` revalidates token boundaries, text and complete consumption.
It preserves source aliases, ignored attribute bits and invisible switches.
`encode` produces a canonical representation, appending an invisible end when
available, then requires that decoding produces the requested raw cell text
without trailing data. It does not promise the original bytes.

`decode_standard` and `encode_standard` use strict WHATWG Shift_JIS
(Windows-31J), through the pinned `encoding_rs` package. This differs from a
minimal JIS repertoire. Standard Shift_JIS encoding has the WHATWG yen,
overline and minus aliases; those aliases can change spelling on decode.
`CharacterMap::encode` detects such spelling changes. JIS X 0201 uses Roman
yen at `5c`, overline at `7e` and half-width kana at `a1..df`, preserving ASCII
control bytes. Undefined bytes and unencodable characters return errors.
No replacement characters are silently introduced.

## Tables and pointer policy

The `tables` object maps names to one of these descriptors:

| `kind` | Fields |
| --- | --- |
| `offsets` | `offsets`, `maximum_bytes`, optional `require_end` |
| `fixed` | `offset`, `count`, `stride`, `length`, optional `require_end` |
| `pointers` | `offset`, `count`, `stride`, optional `pointer_offset`, `pointer`, `maximum_bytes`, optional `require_end` and marker policy |

Integer pointers use `kind: "integer"`, `width` 2 or 4, `endian` (`little`
or `big`), `base` and positive `scale`. Their target is `base + value * scale`.
Far pointers use `kind: "far"` and off16/seg16 in the selected endian order.
Nonoverlapping `windows` supply `minimum`, `maximum`, `base` and optional
`record_bank`. A matching target is
`base + (segment - minimum) * 16 + offset`; `record_bank` also adds the
pointer's containing 64 KiB source bank. Optional `maximum_offset` rejects
noncanonical far offsets. This is a caller-selected file mapping, not CPU
mapper emulation.

Pointer records must fit their stride and the complete source table. Optional
`skip_marker` advances a target only when the marker is followed by a different
byte; a final marker alone remains an empty string. `skip_marker_below` limits
the prelude to targets below that address, distinguishing a pool from another
string region. Unknown segments, out-of-source targets, arithmetic overflow,
truncation and missing required ends return errors. There is no automatic
directory traversal or skip-on-error policy.

`TextLimits` bounds input bytes for each string and total output bytes/tokens
for a table dump. `maximum_strings` separately bounds table population. A
definition does not identify a cartridge revision or discover string starts.
The bundled prefix-bank definition supplies reviewed leaf tables; other maps
accept independently supplied offsets. Directory and script/index tables are
not leaf string tables.

## Worked read/write example

This complete synthetic definition has a fixed two-slot table and an 8x8
1bpp font. The same schema is used for reviewed cartridge definitions.

```json
{
  "schema": "swankit-text-def/1",
  "id": "example",
  "map": {
    "schema": "swankit-charmap/1",
    "entries": [{"bytes": "01", "text": "A"}, {"bytes": "02", "text": "B"}],
    "controls": [{"code": 255, "ends": true}]
  },
  "tables": {
    "labels": {"kind": "fixed", "offset": 0, "count": 2,
      "stride": 4, "length": 4, "require_end": true}
  },
  "fonts": {
    "main": {"offset": 8, "count": 1, "stride": 8, "format": "bpp1",
      "tiles_wide": 1, "tiles_high": 1}
  }
}
```

Load it, dump the table, fit a replacement and extract a glyph:

```rust
use formatkit_core::Result;
use ws_text::{insert_text, TextDefinition, TextLimits};

fn edit(definition_json: &[u8], source: &mut [u8]) -> Result<()> {
    let definition = TextDefinition::from_json(definition_json)?;
    let map = definition.character_map()?;
    let strings = definition.tables["labels"].dump(
        source, &map, TextLimits::default(), 16)?;
    let exact = map.encode_tokens(&strings[0].decoded.tokens, TextLimits::default())?;
    assert_eq!(exact.len(), strings[0].decoded.consumed);
    insert_text(source, strings[0].offset, 4, &map, "AB", 0)?;
    let glyph = definition.fonts["main"].glyph(
        source, 0, &[[255, 255, 255, 255], [0, 0, 0, 255]], 256)?;
    assert_eq!((glyph.image.width, glyph.image.height), (8, 8));
    Ok(())
}
```

For example, the source begins `01 ff 00 00 02 ff 00 00`, followed by eight
synthetic glyph rows. Insertion changes the first slot to `01 02 ff 00`;
the second slot and font are outside that slot. Failed encodings, invalid
offsets and overflow leave the complete source unchanged. The caller must
supply the real slot capacity and handle relocations and checksums separately.

For a reviewed cartridge, select `builtin_definition("byte-prefix-1")` and
the table `table-1002b8`, then use the same `dump` API. That descriptor contains
the source-specific far-pointer windows and pool marker policy. The example
tool accepts the source and definition explicitly:

```sh
cargo run -p ws-text --example dump_table -- \
  byte-prefix-1 "$SOURCE" table-1002b8
cargo run -p ws-text --example inspect_text -- \
  "$DEFINITION_ID" "$SOURCE" "$OFFSET_HEX" "$BYTE_LENGTH" \
  "$FONT_NAME" "$GLYPH_HEX" "$RGBA_OUTPUT"
```

## Font storage

`FontDefinition` requires `offset`, `count`, `stride`, `format`, `tiles_wide`
and `tiles_high`. Formats are `bpp1`, `bpp2`, `bpp4-planar` and `bpp4-packed`.
Each tile is 8x8. `order` is `row` by default or `column`. `tile_offsets` can
select noncontiguous tiles inside a glyph, such as a padded narrow cell;
otherwise tiles are consecutive. These are storage descriptors, not a
universal font file header.

The 1bpp asset format uses one MSB-first byte per row; it is expanded into a
2bpp tile with a zero second plane before rendering through ws-gfx. Hardware
2bpp and 4bpp layouts use the documented display format. The caller's palette
determines RGBA, including alpha; no LCD colour-response model is inferred.

Optional `widths` contains one advance per glyph. `width_table` selects a
resident byte-per-glyph advance table and takes precedence. Missing advances
default to cell width. Advances cannot exceed the bitmap width. These values
do not crop the image or infer proportional bearings. Source extents, output
size, tile offsets and dimensions are checked before indexing or allocation.

## Sources and BIOS fonts

Hardware tile layouts follow [WSdev Tile Data](https://ws.nesdev.org/wiki/Display/Tile_Data).
Encoding policy follows the [WHATWG Encoding Standard](https://encoding.spec.whatwg.org/#shift_jis)
and the [Unicode JIS X 0201 mapping](https://www.unicode.org/Public/MAPPINGS/OBSOLETE/EASTASIA/JIS/JIS0201.TXT).
The latter is one explicit Unicode mapping policy; actual products can differ.
Independent owned Java/Python tools and reviewed tables establish engine
grammar and assignments. The external enrollment records exact oracle
sources, hashes and any oracle preparation corrections. No emulator source
code supplies these rules.

[WSdev FreyaBIOS](https://ws.nesdev.org/wiki/WonderWitch/FreyaBIOS#Fonts)
documents 8x8 alphanumeric and ELISA Shift-JIS fonts in the WonderWitch
cartridge BIOS. This is distinct from the console's internal boot ROM. The
consulted documentation does not establish validated font offsets, storage
ordering or an independent bitmap oracle for either BIOS image. No built-in
BIOS font bank or font bytes are included. A consumer with independently
verified metadata can supply a `FontDefinition` to the generic extractor.
