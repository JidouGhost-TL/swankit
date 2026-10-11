# Asset discovery families

This document describes structural families. Title identities, linked addresses,
ROM bytes, charmaps and screenshots belong in external corpus profiles.
An identical routine proves a shared object, not a shared whole-game engine.
A skeleton fingerprint without operand comparison is a candidate only.

The snapshot adapter and `ws-discover` retain uncertainty. A temporal ROM read /
RAM write association supplies a source candidate. A useful complete decoder
output must equal captured RAM before a compressed source is accepted. This is
not instruction taint tracking. Exact routine-exit witnesses can prove overwritten
outputs; a prefix match cannot. Prior history requires an entry-state capture. Uniform data is excluded from codec evidence. References are checked
against their stored arithmetic and stay incomplete, so relocation is disabled.

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

The existing map codec enum accepts three compression grammars. New codec proofs
are emitted in a discovery sidecar; their stored extents are represented as raw
`other` assets. They are not mislabeled as extractable tiles. A map enum addition
and corresponding `ws-assets` decode/encode dispatch are required to render and
replace these new compressed assets. That owner change is tracked externally.

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
