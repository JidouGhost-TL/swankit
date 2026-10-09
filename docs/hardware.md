# Cartridge hardware rules

Sources consulted for this implementation:

| Document | Rules used |
| --- | --- |
| [WSdev ROM header, revision 680](https://ws.nesdev.org/w/index.php?title=ROM_header&oldid=680) | Footer fields, publisher IDs, declarations, start padding, mapper families and flags |
| [WSdev Memory map, revision 513](https://ws.nesdev.org/w/index.php?title=Memory_map&oldid=513) | CPU ROM windows and one/two-cycle byte access timing |
| [WSdev Mapper, revision 711](https://ws.nesdev.org/w/index.php?title=Mapper&oldid=711) | C0/C2/C3 banking and register widths |
| [WSdev Bandai 2003](https://ws.nesdev.org/wiki/Bandai_2003) | Extended bank registers and RTC interface |
| [WSdev SoC, System Control](https://ws.nesdev.org/wiki/SoC#System_Control_($A0)) | Flag bits 2/3 copied into ROM width/wait-state control |

The final 16 bytes hold EA, little-endian jump offset and segment, maintenance,
publisher, color, game ID, version, ROM size, save code, flags, mapper and a
little-endian checksum. The checksum covers all bytes except its own final two
and wraps modulo 65536. Segment:offset arithmetic wraps to a 20-bit physical
address. A parse requires at least one 64 KiB bank, at most 64 MiB, and EA at
the footer start. Unknown declaration codes remain visible. A checksum or size
declaration mismatch is diagnostic data, since such images occur in the corpus.

Flag bit 0 controls orientation, bit 2 controls width (0 = 8, 1 = 16), and bit 3
adds a wait cycle to the base one-cycle byte access. Width and wait-state flags
are independent. The reference facts generator incorrectly uses bit 1 for width
and bit 2 for speed. Corpus width values happen to agree; one reference speed
value disagrees. The external spec retains the original facts and records the
reviewed speed correction. No source facts are edited. Synthetic tests exercise
all width/wait-state combinations, including combinations absent from the corpus.

Save code 01 needs care: its conventional *nominal declaration* is 64 Kbit, as
used by the existing facts generator. WSdev's current table reports 256 Kbit and
notes that every observed cartridge using code 01 has a 256 Kbit chip, while
explicitly calling for further investigation. `SaveDeclaration` preserves the
nominal 64 Kbit label; it does not claim the physical chip has that capacity.
Callers must not size an actual code-01 SRAM dump solely from this declaration.
Other SRAM and EEPROM capacities follow the documented table. Unknown save
codes do not silently become `None`.

Mapper byte 00 identifies the 2001-compatible family; distinguishing KARNAK
requires external model evidence. Byte 01 identifies the 2003 family and its
RTC interface, without proving that an RTC chip is populated. No filename-based
model guessing or flash-cartridge heuristic is part of this generic parser.

The file size determines banking. Round it up to a power of two, putting any
padding before the file so that the footer remains at the chip's end. Mask chip
addresses with `effective_size - 1`, then subtract the start padding; addresses
in padding have no file offset. Never use modulo the unpadded file length.

Physical CPU addresses 20000..2FFFF and 30000..3FFFF use C2/C3 as 64 KiB bank
selectors. The 2003 high-byte extensions are at D3/D5; C2/C3 alone are 8-bit.
Physical 40000..FFFFF (segments 4000..FFFF with offset zero) uses
`(C0 << 20) | physical_address`. C0 implements four address bits on 2001 and
six on 2003; extended 2003 banks implement ten bits. `BankState::for_mapper`
normalizes those widths. `BankState::RESET` represents the common FF low-byte
state; callers explicitly supply extended high bytes where needed.

Reverse mapping returns all currently visible aliases, since a small ROM may
mirror into several windows and a larger ROM offset may be unmapped. Addresses
outside the 20-bit ROM windows and offsets outside the file return no mapping.

This is an independent Rust reimplementation of the MIT/Apache-licensed header
and cartridge rules in the extension and its facts generator, validated against
the hardware documents above. Mesen, MiSTer, ares, StoicGoose and ARMV30MZ are
behavior references only; their implementation code is not used.

## Display snapshot rules

| Hardware document | Graphics rules used |
| --- | --- |
| [WSdev Display](https://ws.nesdev.org/wiki/Display) | 224x144 LCD, layer order, 32x32 screens, earlier sprite priority and 32 sprites per line |
| [WSdev Tile Data](https://ws.nesdev.org/wiki/Display/Tile_Data) | Row-interleaved planes, MSB-first pixels, packed high nibble first and tile bank locations |
| [WSdev Screen](https://ws.nesdev.org/wiki/Display/Screen) | Tile/palette/bank/flip attribute word |
| [WSdev Sprites](https://ws.nesdev.org/wiki/Display/Sprites) | Four-byte attributes/Y/X records, palettes 8-15, bank 0, priority and window side |
| [WSdev IO Ports](https://ws.nesdev.org/wiki/Display/IO_Ports) | Control, address masks, scroll, inclusive windows, shade pool and palette selectors |
| [WSdev Palette, revision 514](https://ws.nesdev.org/w/index.php?title=Display/Palette&oldid=514) | RGB444, mono darkness, colour-zero transparency and model-dependent colour reproduction |
| [WSdev Frame timing](https://ws.nesdev.org/wiki/Frame_timing) | Delayed sprite-table DMA and palette timing outside snapshot fidelity |

Tiles are 8x8. 2bpp rows have two bytes, one per plane; 4bpp planar rows
have four. Packed rows store two pixels per byte, left pixel in bits 7-4.
Mono 2bpp tiles occupy 2000-3FFF; colour 2bpp bank 1 occupies 4000-5FFF.
4bpp banks occupy 4000-7FFF and 8000-BFFF. Screen entries are little-endian:
bits 0-8 tile, 9-12 palette, 13 colour bank, 14 H flip and 15 V flip.
Sprite bit 12 instead selects the window side, and bit 13 selects priority.
Sprite tile indices always select the first bank.

A mono palette contains four three-bit indices into eight four-bit darkness
levels. Preview grey is `255 - darkness * 17`. Colour palettes contain sixteen
RGB444 entries, expanded by nibble replication. Reserved palette bits are
ignored. Colour zero is opaque for 2bpp palettes 0-3/8-11 and transparent for
4-7/12-15; it is always transparent in 4bpp. Background lookup is opaque.
Neither expansion claims to reproduce a particular LCD panel.

Screen maps start at the address nibble times 800; mono hardware ignores the
high nibble bit. Sprite table addresses use units of 200, with five address
bits on mono and six on colour hardware. Windows have inclusive edges;
reversed edges enclose no pixels. Scroll and sprite coordinates wrap modulo
256. The hardware compositor selects the earliest opaque sprite across both priority
groups; its priority bit then controls screen 2 occlusion.
The hardware profile first selects at most 32 sprites intersecting a scanline,
including offscreen sprites, before priority or window filtering.

### Reference disagreements and limits

The extension's WSAssets 2bpp VRAM lookup masks the tile index to nine bits
even in colour mode, aliasing bank 1 to bank 0. Its WSRender also retains mono
map/table address high bits, selects sprites separately for each priority
group and omits the 32-sprite limit. The explicit
`JavaDiagnostic` profile preserves these behaviors for byte-identical parity;
`Hardware` follows the documented rules. Synthetic tests separate the profiles.
These are compatibility policies, not hardware claims derived from an emulator.

Both profiles render from one RAM instant and optionally per-line port rows.
They do not reconstruct the previous frame's copied sprite table, tile or
palette writes during a line/frame, LCD control timing, segments, rotation,
or panel colour response. The active sprite range is clamped at entry 127,
matching the current diagnostic reference; wrapped or out-of-range hardware
first/count combinations require additional hardware evidence. Static captures
can differ from displayed frames for these reasons. Emulator screenshots are
behavior evidence only and must have scaling, rotation and the icon area
accounted for before comparing. Mesen's panel response is measured externally
with synthetic swatches, never copied from its implementation.

# Sound hardware rules

Sound policy is sourced from [WSdev Sound](https://ws.nesdev.org/wiki/Sound),
[WSdev Hyper Voice](https://ws.nesdev.org/wiki/Hyper_Voice), and
[WSdev DMA](https://ws.nesdev.org/wiki/DMA). Frame-boundary preview spacing uses
[WSdev Timing](https://ws.nesdev.org/wiki/Timing), which specifies 159 lines of
256 clocks by default (318 output frames at 128 clocks each). The sound documents
were checked against the owner's archived hardware documentation. Driver
reconstruction notes supply format grammar, not hardware rules.

Four 16-byte waves hold 32 unsigned 4-bit samples each, low nibble first. Port
8F selects the 64-byte block with address bits 6..13. The 11-bit divider gives
a sample period of `2048 - divisor` CPU cycles at 3.072 MHz. Left/right volume
are the high/low nibble, multiplied by the unsigned sample. Channel 2 can instead
hold an unsigned byte; voice-volume full bits (left 2, right 0) override the
half bits (left 3, right 1). Voice bit 5 operates without channel-enable bit 1.

Sweep adds a signed byte to channel 3's divider modulo 2048 after the selected
number of 375 Hz ticks; an 8D write resets its step counter. Noise uses the
inverted XOR of bit 7 with taps 14, 10, 13, 4, 8, 6, 9 or 11, shifts left and
retains 15 bits. Reset clears the register. The channel-4 and LFSR enable bits
clock it even while wave output is selected. The new low bit represents sample
0 or 15. Noise state ports are read-only, and the reset bit is not retained.

Headphone mixing scales the unsigned channel sums by 32 and wraps additions;
the hardware documentation specifies unclamped adders. Color Hyper Voice has
signed 16-bit outputs, documented extension/shift modes, eight rate divisors
and routing/reset controls. Manual input alternates channels independently of
the DMA layout setting. The ignored extension mode's upper bits are unspecified
and are rejected. The speaker button's exact analog volume steps and sound test
ports are outside the preview model.

Three differences from owner-authored Java references are explicit. A four-byte
driver wave exporter writes the high nibble first and includes a padded half
cycle; ws-sound decodes the documented 32 low-first samples. The Java listening
preview reverses the full/half voice-volume bits and clocks the noise register
only while noise output is selected; ws-sound follows the documented gates.
Reference parity excerpts avoid these differing states. The floating phase and
DC filter in `render_coupled` are retained solely as listening-preview conventions;
they do not specify hardware timing or analog filtering. Default rendering uses
integer phase. The crate README documents further rendering limits.
