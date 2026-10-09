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

## Save media and internal EEPROM

[WSdev EEPROM](https://ws.nesdev.org/wiki/EEPROM) documents cartridge capacities
of 1/8/16 Kbit, internal capacities of 1/16 Kbit, and 16-bit word addressing.
`ws-save` converts these to 128/1024/2048 byte cartridge images and 128/2048 byte
internal images. SRAM geometry is explicit; the nominal ROM declaration does
not determine a physical dump length.

[WSdev Internal EEPROM](https://ws.nesdev.org/wiki/Internal_EEPROM) documents
program data at bytes 00..5F, custom owner-name codes at 60..6F, BCD birthday
at 70..73, gender/blood type at 74/75, previous cartridge metadata at 76..78,
change counters at 7C/7D and startup counter at 7E..7F. Color configuration and
name color are at 83/84. Raw unknown codes and invalid birthday bytes remain
visible. Owner-profile edits preserve counters and unrelated data. Color-only
fields are read only on the explicitly selected color image. No emulator
implementation supplied these offsets or rules.
