# Reviewed text definitions

These files contain character assignments and engine metadata, never cartridge
bytes, actual strings, corpus populations or golden expectations. The IDs
describe encodings so generic consumers do not depend on title names. Source
title mappings, oracle file paths and SHA-256 provenance are recorded in the
external enrollment ledger and milestone report.

| ID | Evidence and represented grammar |
| --- | --- |
| `byte-pages-1` | Reviewed byte-page table and independent Java text/font tools: common cells, three selected pages, paired half glyphs, separate kana marks, byte controls and horizontal/vertical 2bpp fonts |
| `word-dialog-1` | Reviewed word-dialogue table and Java interpreter dump: little-endian glyph words, high-byte controls with conditional operands and horizontal/vertical 4bpp fonts |
| `word-dialog-2` | A separately reviewed word-dialogue interpreter: different operand lengths and additional controls; the two dialogue grammars are deliberately separate data |
| `word-mask-1` | Reviewed merged glyph assignments and measured operand table: masked word indices, retained attribute bits, contextual dash aliases and a 1bpp font |
| `byte-prefix-1` | Reviewed prefix-bank character assignments, pointer-table metadata, independently reproduced Python string dump and font sheets: two prefix banks, narrow cells, separate kana marks and 1bpp fonts |

Literal unknown-cell tags and explicit blank assignments follow the independent
dump conventions. Blank-cell evidence was checked against the source font
outside the repository; no glyph pixels are embedded here. Two paired half-cell
sequences are longest-match entries. Contextual dash aliases allow canonical
encoding while original attributes and code aliases require source tokens.

Only reviewed leaf string tables are included in `byte-prefix-1.tables`.
Directory, script and index tables require their own higher-level grammar.
Other definitions supply maps and fonts; callers provide their reviewed string
starts using an offsets table. Static/runtime discovery evidence stays outside
these definitions.

Font widths here describe cells. Proportional advances are supported by the
generic API but are not inferred from glyph pixels or substituted for the
source interpreter's bearing calculation. Every bank requires an explicit
palette. See [hardware and encoding sources](../../../docs/text-definitions.md#sources-and-bios-fonts).
