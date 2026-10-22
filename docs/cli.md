# Command line

`wg-adapter` and `wg-decode` also build as stand-alone binaries for emulator
excomm use and tap inspection. The matching `swankit wg-adapter` and
`swankit wg-decode` commands share their implementations. See
[WonderGate raw mode](wondergate.md#raw-executables) for arguments, environment
configuration, EOF behavior and the lossless byte log.

Build the private binary with `cargo build -p swankit-cli --locked`. The
examples assume the resulting `swankit` is on PATH. Use `--help` for syntax
and `--version` (or `-V`) to print the package version.
Hardware interpretation comes from the owner crates and
[hardware sources](hardware.md); extraction definitions supply offsets and
formats, never game detection from a filename.

`swankit link` relays binary serial streams; see [link endpoints and recipes](link.md).
Its stdout is raw bytes when using `stdio`, and its TSV log requires a new file.

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

## Mapped assets

Replacement needs a ROM of at least 64 KiB (65,536 bytes); smaller mapped
buffers can verify and extract. It rewrites the final two bytes with the
little-endian sum of all preceding bytes modulo 65,536. It does not validate every footer
field or require a correct input checksum.

```sh
swankit assets verify cartridge.wsc assets.json --json
swankit assets extract cartridge.wsc assets.json extracted
swankit assets replace cartridge.wsc assets.json --id message \
  --from extracted/message.txt --text-framing lf \
  -o edited.wsc --map-out edited-assets.json
```

Each mapped text `.txt` is one decoded string plus one framing LF, including
empty strings and strings containing CR or LF. Default `.txt` replacement
removes one optional final LF and then one optional CR, even without an LF.
This preserves legacy CRLF handling but can drop a content CR. The explicit
`--text-framing lf` mode removes only one optional final LF and preserves all
remaining CR and LF characters; without a final LF, it keeps the whole string.
Retain the framing LF when editing, including after a content LF or CRLF.
Other values and uses outside `.txt` text replacement are refused before writes.

PNG supplies supported image edits. Files with other extensions, such as
`.bin`, supply decoded payload bytes directly, including encoded character and
control bytes and terminators; they must omit `--text-framing`. Renaming a text
export is not a conversion to those bytes. Compressed assets are re-encoded by
their codec. Keep the output ROM with its returned map. See [asset maps and
replacement](asset-maps.md) for text framing examples, code unit tables,
allocation, relocation, layout limits and output safety.

## Codec extraction

```sh
swankit extract codec cartridge.wsc --def codec.json --output decoded.dat --json
```

A `swankit-codec-def/1` definition selects one headerless stream. For example:

```json
{"schema":"swankit-codec-def/1","codec":"lz-rle-7f",
 "offset":4096,"length":128,"max_output":512}
```

`codec` is `lz-rle-7f`, `lz-bits-16`, `lz-u8-zero`, `lz-word16-msb`, `okumura-lzss`, `lzss-u16-zero`
or `lzss-12`. `lz-u8-zero` uses literal blocks and backward matches terminated
by a zero command; its consumed length includes that command and excludes any
following carrier bytes. The length-prefixed grammar accepts `destination` (default 8192)
and `prehistory` (`low5-prefix`, `low8-prefix`, `zero-until-start`). The
external-length grammar requires `decoded_size` and accepts explicit `history`. `offset` defaults to
zero; `length` and `max_output` are required. The output bound is at most
65,536 bytes. Optional `history` is an array of known preceding RAM bytes for
the terminated grammars. The library reports consumed stream bytes through
the terminator; trailing carrier data remains available. JSON artifact metadata
includes `consumed`.

`lz-word16-msb` starts at its mode byte. Mode zero reads a little-endian u16
literal count and those bytes. A nonzero mode uses little-endian flag words,
consumed most-significant bit first with refill only before the next command.
It refuses external history. Its extent ends after the counted payload or
zero match token; any carrier prefix belongs outside the definition's offset.

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

For a packed wave at the beginning of a caller-owned file, a minimal sound
definition is:

```json
{"schema":"swankit-sound-def/1","format":"wave","offset":0,"length":16}
```

`extract sound` returns 32 values in 0..15 per wave, low nibble first. For
unsigned PCM, select `"format":"unsigned-pcm"`, set the exact byte `length`
and a positive `sample_rate`; samples become signed 16-bit values using
`(n - 128) * 256`. The rate is supplied by the caller, not detected from bytes.
These operations export JSON; Python's `Pcm.s16le()` supplies raw PCM bytes.

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

## WonderGate server

```sh
swankit wg-server --data DATA_DIR --wire WIRE_DIR --bind 127.0.0.1 \
  --http-port 8080 --bpl-port 5555
swankit wg-server --data TEST_DATA --wire TEST_WIRE \
  --clock fixed:20261005200403 --seed 123
```

`--data` and `--wire` are required; `--logs` is an alias for `--wire`.
The numeric bind address defaults to loopback and ports default to 80/5555.
Port zero requests an ephemeral port; startup prints both actual addresses.
The clock defaults to system UTC; fixed-clock digits are UTC, with gettime
converted to JST. Entropy defaults to the Unix OS source; an unsigned 64-bit
decimal seed opts into deterministic test tokens. Records append to `DATA_DIR/journal.jsonl`, and
lossless transactions append to `WIRE_DIR/wire.jsonl` with raw byte sidecars.
Terminate the owned process with the normal operating-system signal. Each
journal record is fsynced before service success. Completed wire entries are
fsynced before the HTTP connection closes or the next BPL message is read.
The command loads bundled shared services and validated title definitions.
Phase synchronization, empty check-in acknowledgments, newly submitted rankings,
static stock and explicit 501 content refusals are data-driven. `--data` stores
the journal; it does not select definition files. Edit shipped JSON and rebuild
to change CLI policy, or use `Server::from_definitions` in an embedding application.
See [the definition read/write guide](wondergate-title-definitions.md) and
[the server API](../crates/ws-wondergate-server/README.md).

## Trace-driven draft discovery

`swankit assets discover ROM EVIDENCE -o MAP --report REPORT` consumes normalized
`swankit-discovery-evidence/1` data. `--force` permits output replacement; `--json`
prints the coverage report. `swankit assets discover --help` lists the arguments.
`tools/discover-assets.py` prepares snapshots, runtime source groups, decompile
references and explicitly selected external family profiles. See
[asset families](asset-families.md) for the data contract and limits. A verified
draft preserves uncertainty and does not establish exhaustive references or
free space. New grammar witnesses outside the current asset-map codec enum
remain labelled in the report sidecar.

`swankit tiles SNAPSHOT --snapshot --line-ports PORTS --output PNG` accepts
144 consecutive 256-byte port rows, one per visible scanline. It composes those
rows over the snapshot's single RAM image with the existing `ws-gfx` renderer.
This captures scroll/window/register changes; intra-frame palette RAM and tile
updates still require additional RAM evidence. Use `--color` for a color hardware
capture, independently of the saved value of a mono-only/unavailable register.
`tools/capture-scenes.lua` can capture existing states, input replays, RAM, ports,
screenshots and (with `RASTER=1`) scanline ports through the emulator debugger API.
Its environment inputs are described in the script; assets remain external.

## Offline distributions

The complete packet includes this guide, schemas and linked crate guides.
See [offline installation and first operation](distribution.md) for platform
requirements, artifact/source binding and a synthetic tile example. Use exact
supported command paths such as `swankit extract sound --help` for syntax.
Unknown actions and malformed option requests return an error.
