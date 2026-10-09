# ws-save

Checked save media, owner-profile fields and parameterized checksum arithmetic.
Byte reads, fields and edits use Formatkit; the crate performs no filesystem I/O.
All workspace crates inherit `publish = false`.

`SaveImage::parse` requires an explicitly selected `Media` and its exact length.
SRAM sizes are supplied separately from ROM declarations. Cartridge EEPROM is
128, 1024 or 2048 bytes; internal EEPROM is 128 bytes on the mono console and
2048 on color consoles. `word(address)` uses byte offset `2*address` and rejects
SRAM. `write_word` checks geometry and bounds before editing.

`OwnerProfile` preserves raw owner codes and BCD birthday bytes alongside decoded
values; unknown character codes remain visible and invalid dates decode to None.
Writes update stored name codes, raw birthday, gender and blood type, preserving
counters and reserved/splash data. See [save hardware rules](../../docs/hardware.md).

The `checksum` module exposes `Rule` (byte/word sums, XOR, rotations and weighted
sums), reflected `Crc16`, final negation, XOR/sum pairs, lane sums, rolling pairs
and checked word permutations. All seeds, widths, masks and endian choices are
explicit. Game-specific coverage and repair policy are in
[JSON save definitions](../../docs/save-definitions.md).
