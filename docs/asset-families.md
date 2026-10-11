# Asset discovery families

This document describes structural families. Title identities, linked addresses,
ROM bytes, charmaps and screenshots belong in external corpus profiles.
An identical routine proves a shared object, not a shared whole-game engine.
A skeleton fingerprint without operand comparison is a candidate only.

The instruction collector `tools/ghidra/WSAssetTrace.java` wraps only the
installed extension's private emulator instance and delegates every hardware
callback. It propagates byte origins through executed loads, stores, register
copies, integer operations and GDMA transfers. Monochrome RAM aliases share
provenance cells; CPU stores crossing the RAM boundary do not wrap. Call witnesses include entry
registers, bank settings for direct ROM reads, source origins carried through
earlier registers/RAM, physical destination spans, CPU destination addresses and bytes captured at
return. Run it through your isolated headless runner with arguments
`OUTPUT FRAMES INPUT_KEYS 0`; `INPUT_KEYS` contains `first last hexmask` rows.
The optional fifth argument `self-test` runs byte-provenance regressions instead.
`WSAssetTraceCarryChecks.java OUTPUT` adds cross-call, RAM alias and boundary
regressions. Inherited RAM origins retain ROM file offsets but do not invent the
bank settings of an earlier call.
The collector targets the pinned Ghidra callback API; it never modifies the
installed extension.

`tools/discover-assets.py --instruction-flow OUTPUT/calls.jsonl` normalizes
complete return witnesses without a temporal fallback. An exact complete decode
must equal those captured bytes before a codec is accepted. `--flow` remains a
legacy candidate input for reproducibility. The data-flow collector does not
propagate branch control dependence, IRQ/DMA side effects outside the recorded
callbacks, sound DMA, or arbitrary userops. Unknown output operators and origin
sets over 64 bytes become opaque. Register/address origins can include constants,
table indices and banks; they are reference candidates until field arithmetic
and reader semantics are independently reviewed. They never authorize relocation.
`source_ranges` lists the exact half-open ROM origin ranges contributing to a
span; `source`/`source_end` bound all ROM data read in the call and remain the
codec probe context. That context can contain unrelated reads. Data provenance
omits compression flag/control dependence, so causal ranges alone are not a
complete stored stream extent.
Uniform data and incomplete calls are not codec proof. Supplied prior history
still requires an entry-state capture.

`tools/discover-scenes.py ROM CAPTURES.json --binary SWANKIT -o OUT` accepts a
`swankit-scene-captures/1` manifest with a `scenes` array of `id`, `ram`, `ports`,
optional `instruction_flow` and optional `profile` paths (relative to the manifest).
It writes and verifies a separate asset map per scene, preserving each scene's
palette and tile placements. Its summary counts overlapping ROM ranges once and
counts text offsets and reference fields once. The scene VRAM metric sums the
per-scene denominators; compare a fixed single-frame selection separately.
Hardware snapshot composition includes OAM sprites; it does not establish the
ROM animation or frame-table grammar.

| Family | Recognizing evidence | Storage and loader policy |
| --- | --- | --- |
| Length-prefixed backward LZSS | u16 output length; eight LSB flags; literal bit 1; little-endian token distance = word >> 4, length = (word & 15) + 3 | Banked source, RAM destination; zero prehistory dialects use a five-bit counter, an eight-bit counter, or fill until the original destination; no absolute ring buffer |
| External-length backward LZSS | eight LSB flags; literal bit 1; little-endian token distance = (word & 4095) + 1, length = (word >> 12) + 3 | Carrier owns size, pending source/destination/count, and resumable transfer policy; explicit prior destination history if required |
| Byte-command resource loader | bank plus source offset, destination units, target flags, and compression bit | Existing `ws-lz-rle-7f`; maps and palettes may have different unit counts |
| Word-flag resource loader | eagerly refilled 16-bit LSB flags and short/long backward matches | Existing `ws-lz-bits-16`; distinguish from byte flags by code and output equality |
| Raw banked copies | ROM bytes equal final tile/map/palette bytes; loader read source corroborates location | Raw snapshot matching plus incomplete immediate references; a bank window is not a file address |
| One-plane glyph expansion | six or more consecutive eight-byte records equal expanded two-plane VRAM tiles | 8x8, 1bpp candidate storage; meanings, font/table extents and widths need separate review |
| Far-pointer text records | offset16 + segment16, paragraph or fixed-segment interpretation proven by reader | External profile defines record stride, pointer offset, window, terminator and charmap |
| Fixed word glyph records | Reviewed word charmap and reader; two-by-two 4bpp glyph tiles | 16x16 planar glyphs, separate orientation sheets; attributes and controls stay token-preserving |
| Masked word glyph records | Reviewed word charmap and reader; two-by-two 1bpp glyph tiles | 16x16 column-major masks; glyph identity and palette attributes are separate |
| Prefix-bank glyph records | Prefix byte chooses glyph bank/geometry; terminator and control grammar reviewed | 1bpp 8x8, 8x16 and 16x16 records; a bitmap does not prove a width table |
| Shared warning bitmaps | Exact copied tile blocks across independently linked binaries; visually inspected | 2bpp 8x8 tiles containing warning-message fragments, not an alphabet font or shared script engine |
| Shared sound object | identical routine bytes, writes to sound registers, same channel-state strides | Driver family evidence does not establish script/text/graphics compatibility |
| Shared display object | identical routine bytes and display/OAM register writes | Initializer/window helper evidence does not establish a game asset-engine family |

`ws-lzss-u16-zero` and `ws-lzss-12` are clean-room implementations of independently
inspected cartridge routines. Their encoders use ordinary backward matches and
avoid dependence on uninitialized destination memory. Both reject output
overshoot and malformed history. Corpus records cite the original addresses,
instruction listings, capture hashes and complete RAM comparisons.

## Re-running discovery

Run `tools/discover-assets.py ROM --ram RAM --ports PORTS --flow FLOW --decomp DIR
--profile PROFILE --binary SWANKIT -o MAP`. The profile is optional and is selected
explicitly. It must match the ROM SHA-256. No title names select generic behavior.
The tool emits normalized `swankit-discovery-evidence/1` JSON and calls
`swankit assets discover ROM EVIDENCE -o MAP --report REPORT`. The Rust core checks
identity, probes codecs against snapshots or exact routine-exit witnesses, imports reviewed profile text,
declares overlaps and verifies the entire draft with `ws-assets`. Only declared snapshot
tilemaps receive newly matched VRAM placement; reviewed scene relations remain explicit.

Profiles contain declarative asset geometry, inline charmaps, traced source
observations and `text_tables`. A text table declares its offset, count, stride,
pointer encoding, pointer field offset, address window, terminator, code-unit
width and maximum string length. Far pointers additionally declare segment base
and normalization. The adapter checks bounds; `ws-text` performs text parsing
and `ws-assets` validates reference arithmetic.

A low-confidence read-backed ASCII candidate is not a translated script.
Strings found by scanning plausible Shift-JIS bytes without a reader or font
relationship are not promoted. Font expansion candidates prove bitmap storage,
not typography. Data profiles must preserve this distinction.

## Current map boundary

The map accepts all five compression grammars, including `ws-lzss-u16-zero` with
an explicit destination and prehistory policy, and `ws-lzss-12` with a decoded
size and explicit prior history. Discovery retains immutable raw stored-byte
sidecars and adds editable decoded assets for complete witnesses. Earlier maps
remain valid baselines; a sidecar alone does not represent an editable tile set.

No free-space ledger is synthesized. Executed/data-read CDL alone cannot prove
that an unvisited byte is free. Multi-scene captures, routine entry/exit witnesses,
DMA source/destination tracing, full instruction data-flow and reviewed per-family
character maps remain necessary for complete maps.

Hardware placement follows the cited [memory/mapper rules](hardware.md) and
[graphics display layouts](../crates/ws-gfx/README.md). Emulator captures are
behavioral oracles; no emulator implementation is copied.

The measured snapshot metric is the fraction of unique visible tile slots whose
complete tile bytes occur in decoded mapped payloads. Shared slots across layers
count once. It includes blank tiles, omits OAM sprites and sub-tile scroll edges,
and measures explained storage rather than a fully rendered scene. Sidecar codec
witnesses are reported separately from assets the current map enum can render.
The runtime association does not recover every true destination; explicit
entry/exit or reviewed descriptor data can resolve those misses.
For destination-sensitive header streams, a failed temporal hint is followed
by one aligned probe. Acceptance still requires the whole output to match and
decoding again at the actual located destination to produce identical bytes.

The prehistory policies preserve distinct control flow. The five-bit prefix
loop stops on its counter or token length. The eight-bit prefix loop also stops
when the source reaches the original destination. The fill-until-start loop
uses that destination boundary without a prefix counter. An unaligned
destination therefore distinguishes policies even when aligned fixtures agree.

Frozen discovery maps are verified baselines rather than complete inventories.
Corpus replay preserves every baseline asset, relation, reference, map field and
overlap declaration; it permits additional verified assets and overlap links.
The original evidence and baseline files are never overwritten as recognition
improves. Independent decoded-output hashes remain separate corpus expectations.

New reference candidates remain incomplete; reviewed closure supplied in a
profile is retained. An arithmetic match checks stored values, while routine
entry/exit evidence establishes execution context. Decoder destination
parameters denote the original segment offset. Segmented observations must
distinguish that offset from linear RAM placement; bank aliases and DMA origins
need explicit evidence rather than an inferred history buffer.

An observation may supply `codec_destination` (u16) separately from the linear
RAM `destination`. It is the decoder's original offset register. Explicit
register evidence disables the legacy aligned fallback and is retained in the
map codec parameters. The instruction adapter infers it only when entry DI and
DS or ES reproduce the captured span's CPU start before physical RAM aliasing. Other register conventions
require a reviewed profile, rather than a guessed DI.
