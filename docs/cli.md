# Command line

Build the private binary with `cargo build -p swankit-cli --locked`. The
examples assume the resulting `swankit` is on PATH. Use `--help` for syntax.
Hardware interpretation comes from the owner crates and
[hardware sources](hardware.md); extraction definitions supply offsets and
formats, never game detection from a filename.

`--json` prints machine results on stdout. Errors are one JSON object with an
`error` string on stderr and exit status 1; plain mode prints readable errors.
Detection of unknown or ambiguous bytes is a successful inspection, represented
by `selected: null` and a candidate list. A file read error aborts the command.
Binary artifacts require `--output`; text and sound can write JSON to
`--output` or display on stdout. Existing outputs require `--force`; use a
separate destination to preserve the source. Paths and command arguments must
be UTF-8; use `--` before positional paths beginning with `--`. Numeric options
accept decimal or `0x` hexadecimal. JSON numbers are decimal.

## Cartridge facts and detection

The experimental `interop --json [--server]` command is a separate versioned
JSON-lines protocol for Java evaluation. Request errors appear in replies on
stdout, and a recoverable request error does not fail the process. See the
[interop contract](../crates/swankit-ffi/README.md) for framing, operations,
buffer limits and child lifecycle requirements.

```sh
swankit info cartridge.wsc
swankit info cartridge.wsc --json
swankit detect cartridge.ws save.eeprom update.ips --json
```

`info` requires the composed ws-rom probe, then reports footer fields, publisher,
reset address, orientation, bus width, ROM cycles, save declaration, actual and
declared sizes and checksum status. Checksum and declared-size mismatches remain
visible facts. The JSON contains numeric codes rather than presentation hex.

`detect` classifies by owner content probes. ROM and IPS checks do not trust
extensions. Headerless codecs, tiles, text and driver sequences have no global
probe and need a selected definition. Candidate lists and confidence labels are retained to expose
ambiguity. Save media interpretations and semantic definition candidates are
reported separately; a size match alone cannot authorize an automatic edit. Individual file reads are capped at 64 MiB and definitions at 4 MiB.

## Tiles and snapshots

```sh
swankit tiles cartridge.wsc --offset 0x1000 --length 0x400 \
  --format 2bpp --columns 16 --scale 2 --output tiles.png
swankit tiles display.snapshot --snapshot --color --output screen.png
swankit tiles display.snapshot --snapshot --profile java --output diagnostic.png
```

Atlas formats are `2bpp`, `4bpp-planar` and `4bpp-packed`; offset and length
select complete source tiles. `--length` and `--format` are required. Columns
default to 16 and scale to 1. Atlases use an explicit diagnostic grayscale
palette (index zero white), not a guessed ROM palette.

A snapshot is exactly 65,536 RAM bytes followed by 256 port bytes. `--color`
selects color hardware; absence selects monochrome. The default `hardware`
profile and optional `java` diagnostic profile preserve ws-gfx's documented
policy differences. Snapshot rendering uses the actual palette in the snapshot.
RGBA allocations are capped at 64 MiB. The CLI's small PNG writer emits RGBA8,
filter zero, noninterlaced images with stored DEFLATE blocks according to the
[PNG specification](https://www.w3.org/TR/png-3/); it favors simple, valid output
rather than compressed file size. No PNG dependency enters the product library.

## Text and fonts

```sh
swankit extract text cartridge.wsc --def text.json --table dialog --json
swankit extract text cartridge.wsc --def text.json --table dialog --output text-dump.json
swankit extract font cartridge.wsc --def text.json --font main --columns 16 --output font.png
```

`--def` accepts a `swankit-text-def/1` JSON path or one of ws-text's neutral
bundled IDs. Named tables and fonts follow [text definitions](text-definitions.md).
The JSON text dump retains offsets, raw and normalized text, consumed bytes,
termination, page state and exact source tokens. Plain output is offset plus
raw text. Font atlases use full glyph bitmaps, grayscale and transparent unused
cells; definition advance widths do not crop bitmaps. Columns default to 16.
Text extraction limits are ws-text's bounded default input, output and token
limits and a maximum of 65,536 strings.

A synthetic definition for the byte `41` followed by a zero terminator:

```json
{
  "schema": "swankit-text-def/1",
  "id": "example",
  "map": {
    "schema": "swankit-charmap/1",
    "entries": [{"bytes": "41", "text": "A"}],
    "controls": [{"code": 0, "ends": true}]
  },
  "tables": {
    "dialog": {"kind": "fixed", "offset": 0, "count": 1,
               "stride": 2, "length": 2, "require_end": true}
  },
  "fonts": {
    "main": {"offset": 16, "count": 1, "stride": 16,
             "format": "bpp2", "tiles_wide": 1, "tiles_high": 1}
  }
}
```

## Codec extraction

```sh
swankit extract codec cartridge.wsc --def codec.json --output decoded.dat --json
```

A `swankit-codec-def/1` definition selects one headerless stream. For example:

```json
{"schema":"swankit-codec-def/1","codec":"lz-rle-7f",
 "offset":4096,"length":128,"max_output":512}
```

`codec` is `lz-rle-7f`, `lz-bits-16` or `okumura-lzss`. `offset` defaults to
zero; `length` and `max_output` are required. The output bound is at most
65,536 bytes. Optional `history` is an array of known preceding RAM bytes for
the two terminated grammars. The library reports consumed stream bytes through
the terminator; trailing carrier data remains available. JSON artifact metadata
includes `consumed`.

Okumura requires `decoded_size`; optional `bit_order` is `lsb-first` (default)
or `msb-first`, and `fill` defaults to zero. It uses ring fill and rejects an
external `history`. All definitions reject unknown fields and schema versions.

## Sound extraction

```sh
swankit extract sound cartridge.wsc --def sound.json --json
swankit extract sound cartridge.wsc --def sound.json --output sound-dump.json
```

A `swankit-sound-def/1` definition selects `wave`, `unsigned-pcm`,
`sdk-sequence`, `cell-sequence` or `byte-sequence` and a bounded extent:

```json
{"schema":"swankit-sound-def/1","format":"sdk-sequence",
 "offset":8192,"length":256}
```

Wave extraction returns arrays of 32 four-bit samples per complete 16-byte
wave. Unsigned PCM requires an explicit positive `sample_rate` in the definition
and returns signed sample values at that source rate, without a WAV wrapper. Sequence extraction returns structured decoded
records, offsets and consumed extents, stopping at the first terminator.
It does not execute calls, loops, bank changes or driver mode transitions.
A missing terminator, truncated record or unsupported opcode is an error.
Sequence work is capped at the owner's record limit.

Byte sequences additionally accept `dialect` (`a`, `b` or `c`, default `a`),
`direct`, `direct_divisor`, `channel` and `call_depth_bytes` (all default zero
or false). Those fields are the explicit input mode for the owner instruction
parser. Definitions reject unknown fields and schema versions.

## IPS patches

```sh
swankit patch create original.dat edited.dat --output edit.ips
swankit patch apply edit.ips original.dat --output patched.dat
swankit patch apply update.ips cartridge.ws --overlap last-write-wins \
  --growth zero-fill --output patched.ws --json
```

Default application rejects overlaps and growth. Select last-write-wins and
zero-fill explicitly when independently reviewed patch policy requires them.
The owner parser checks RLE, record extents and optional EOF size extensions;
output is capped at 64 MiB. Creation uses the existing deterministic ws-patch
encoder. Truncation and size extensions follow the owner's documented policy.

## Save commands

The default `saves` feature includes both save owner crates. Build the binary
with `--no-default-features` to omit save operations; requests then return a
clear feature error. Detection reports possible media by size and matching
bundled definition IDs with validity evidence. Size alone does not establish a
physical medium or title. Definitions share sizes and signatures, so `auto`
requires exactly one positive candidate and reports ambiguity otherwise.

```sh
swankit save decode save-definition.json save.eeprom --json
swankit save decode auto save.eeprom --json
swankit save set save-definition.json save.eeprom --field 0 --value 7 --output edited.eeprom
swankit save fix save-definition.json damaged.eeprom --output repaired.eeprom --json
swankit detect save.eeprom --context context.json --json
```

Definitions are external `swankit-save-def/1` JSON files or bundled IDs read
from those definitions. Decode returns the owner's complete JSON tree, including
invalid and empty record flags. It does not invent a successful semantic state
from a parse. See [save definitions](save-definitions.md) for the full schema.

`set` selects a stored field by its zero-based index; `--value` is a JSON scalar
or byte array matching the stored encoding. It edits a copy and invokes the
owner's transactional repair. `fix` invokes that repair directly. Both require
a usable post-repair record according to the definition's program and excluded
working regions, preserve the image length and write only after success.
Some definitions have no meaningful protection rewrite; their owner repair
rules apply. Fixed-field edits do not interpret arbitrary semantic encodings
or choose another image for an internal/companion field. Select the appropriate
image and field using the definition metadata. JSON artifact details include
the definition ID and all returned verification flags; invalid empty redundant
records may coexist with a usable stored record.

Optional `--context` explicitly supplies lookup tables and companion images:

```json
{"schema":"swankit-save-context/1","tables":{"lookup":[0,1,2]},"images":{}}
```

`tables` and `images` default to empty objects of byte arrays; unknown fields,
schema versions and out-of-range bytes are errors. Image keys are the exact
logical names requested by the definition program. The library reads no host
files for companions. `--name` overrides the logical input name (by default the
provided source path), preserving filename-keyed results and companion naming
when a staged input has a different filename. Detection accepts `--context`
and, for one input only, `--name` too. Auto selection uses those same explicit
inputs and ignores candidate parse errors; an execution error from a selected
definition is reported directly.

## Verification

`tools/ci.sh` runs synthetic CLI smoke tests and the independent PNG check.
Product corpus enrollment reuses the existing frozen owner facts and hashes,
verifies source hashes first and creates a new selection without changing them:

```sh
python3 tools/stage-corpus.py --product-detection --output "$STAGING_DIR"
python3 tools/stage-corpus.py --product-saves --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

Product and binary corpus handlers compare detections and ROM facts with those
external expectations through Formatkit's serialized safe runner. Corpus data,
population counts and golden hashes remain outside Git.
