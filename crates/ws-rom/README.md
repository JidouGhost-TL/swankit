# ws-rom

Checked cartridge footer parsing, checksum facts, nominal ROM/save declarations,
publisher lookup and banked CPU address mapping. Formatkit supplies the byte
reader, configured fields and catalog types. No ROM bytes are retained.

```rust
use ws_rom::{BankState, RomFacts};

# fn inspect(bytes: &[u8]) -> formatkit_core::Result<()> {
let facts = RomFacts::parse(bytes)?;
let h = facts.header;
let reset = h.reset_linear();
let banks = BankState::RESET.for_mapper(h.mapper())?;
let file_offset = facts.geometry.address_to_file_offset(reset, banks);
let aliases = file_offset.map(|offset| facts.geometry.file_offset_to_addresses(offset, banks));
# let _ = aliases;
# Ok(())
# }
```

`Header::parse` validates size and the far-jump opcode. Unknown declaration
codes remain explicit; a checksum or declared-size mismatch stays reportable.
`RomGeometry` uses actual file size, start padding and power-of-two mirroring.
`catalog::ROM_FORMAT` declares the `ws-rom` identity for explicit composition
by the later product crate. The catalog uses bounded footer validity checks,
and deliberately avoids a full-image checksum during detection.

The external corpus harness checks independent facts, reviewed reference
corrections, SHA-256 digests and catalog probes. Ordinary tests skip external
corpus work. See the workspace's hardware documentation for sources, flag
corrections and the unresolved distinction between save-code-01 nominal and
physical capacity. Future generic and game crates inherit the same workspace
conventions; product dispatch remains a later milestone.
