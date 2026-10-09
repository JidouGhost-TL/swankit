# ws-compress

Private owner grammars for headerless WonderSwan compression. Select a codec
from the containing format; the catalog deliberately supplies no byte probes.
Both terminated decoders return Formatkit's `DecodeOutcome`, reporting only the
bytes through the terminator. Trailing carrier data remains unconsumed.

| Module | Grammar | Encoder |
| --- | --- | --- |
| `lz_rle_7f` | 10-bit backward distance, literals, zero-prefixed words, runs, zeros; `7f ff` terminator | Minimum encoded-byte dynamic programming |
| `lz_bits_16` | LSB-first 16-bit little-endian flags, short/long matches, extended zero-length terminator | Reference-compatible minimum-bit dynamic programming with bounded match search |
| `okumura` | 4 KiB initialized ring, flag bytes, external decoded length | Formatkit's original tree matcher |

```rust
let raw = b"abcabcabc";
let stored = ws_compress::lz_rle_7f::encode(raw)?;
let decoded = ws_compress::lz_rle_7f::decode(&stored, raw.len())?;
assert_eq!(decoded.bytes, raw);
assert_eq!(decoded.consumed, stored.len());
# Ok::<(), formatkit_core::Error>(())
```

Every decoder takes an explicit output limit, checks complete token growth,
and reports errors on truncation or invalid references. The terminated formats
support at most 65536 decoded bytes. Their encoders reject larger input before
allocating parse tables. Optional `decode_with_history` takes the known RAM
bytes preceding the destination; unknown prehistory is never invented.
Matches may overlap their own output. `lz_bits_16` fetches the next flag word
immediately after consuming bit 16, before that token's operand bytes.

Okumura delegates the complete mechanism to `formatkit-codec`, with explicit
bit order, ring fill, output size and limit. It rejects token overshoot rather
than silently clipping a back-reference to the declared size. No header or
padding convention is guessed.

Encoder output is deterministic. The byte-command encoder follows the
independent packer's tie order. The word-flag encoder minimizes the reference
bit cost, which does not guarantee minimum physical byte count after flag-word
rounding; at most 512 prefix candidates are considered at a position. Placement
and original-slot fit remain the containing format's responsibility.

Grammar evidence is the project's independent asset-tool implementations and
captured stream/RAM pairs. Hardware policy and segment/bank placement belong
to the carrier; see [hardware sources](../../docs/hardware.md). No emulator
implementation is used. Synthetic tests cover all commands, eager refill,
history, malformed streams and bounds. External frozen specs compare Java
decode extents, RAM captures, encode/decode round trips, encoder bytes and fit.
