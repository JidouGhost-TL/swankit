# ws-compress

Private owner grammars for WonderSwan compression. Select a codec from the
containing format; the catalog deliberately supplies no byte probes.
Terminated decoders return Formatkit's `DecodeOutcome`, reporting only the
bytes through the terminator. Trailing carrier data remains unconsumed.

| Module | Grammar | Encoder |
| --- | --- | --- |
| `lz_rle_7f` | 10-bit backward distance, literals, zero-prefixed words, runs, zeros; `7f ff` terminator | Minimum encoded-byte dynamic programming |
| `lz_bits_16` | LSB-first 16-bit little-endian flags, short/long matches, extended zero-length terminator | Reference-compatible minimum-bit dynamic programming with bounded match search |
| `lz_u8_zero` | Byte commands: 1–127 literal bytes, 3–130 byte matches with distance 1–256, zero terminator | Minimum encoded-byte dynamic programming |
| `lz_word16_msb` | Mode0: u16LE count/literals; nonzero: lazy MSB-first u16LE flags, distance12/length4 tokens, optional length extension, zero-word terminator | Counted literals through65535 bytes, literal-only flags for65536 |
| `lzss_u16_zero` | u16 length, LSB byte flags, length4/distance12 tokens, destination-dependent zero prefix | Minimum-byte parse over literals and backward matches |
| `lzss_12` | External length, LSB byte flags, distance12-minus-one/length4 tokens | Deterministic backward-match encoder |
| `lzss_12_ext` | External size, LSB byte flags with zero literals, distance12-minus-one, length4 plus two or extension plus eighteen, clamped final match | Decode only |
| `flag16_distance11` | Mode byte and u16LE output count; nonzero mode uses LSB-first u16 flags, zero literals, distance11-plus-one/length5-plus-three matches | Decode only |
| `mask16_fill` | Nonzero u8 group count; each group has a fill byte and LE16 mask, low then high byte MSB-first; one emits fill, zero consumes a literal; 16 outputs per group | Decode only |
| `lz_command16_lsb` | BE16 command count; lazy LSB-first flags select a literal, literal run, or distance/length backreference | Decode only |
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
Matches may overlap their own output. `lz_u8_zero` distinguishes its zero command
from literal zero bytes and uses no implicit initial fill. `lz_bits_16` fetches the next flag word
immediately after consuming bit 16, before that token's operand bytes.

`lz_word16_msb` starts at a mode byte and excludes carrier prefixes. Mode zero
copies a little-endian u16 count of literal bytes. Any nonzero mode reads
little-endian flag words MSB first, refilling lazily before the next command.
One means literal; zero means a little-endian token, with zero terminating.
Other tokens have distance `4096 - (token >> 4)` and length `(token & 15) + 3`;
low bits 15 add a further unsigned byte. Overlapping copies use produced bytes
only. Encoding emits counted literals through 65,535 bytes and literal flags
at 65,536 bytes, with no compression or original-slot fit guarantee.

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
captured stream/RAM pairs. The zero-terminated byte grammar is independently
transcribed from cartridge instructions and checked against complete captured
destination buffers. Hardware policy and segment/bank placement belong
to the carrier; see [hardware sources](../../docs/hardware.md). No emulator
implementation is used. Synthetic tests cover all commands, eager refill,
history, malformed streams and bounds. External frozen specs compare Java
decode extents, RAM captures, encode/decode round trips, encoder bytes and fit.

Additional explicit extraction definitions accept `lzss-u16-zero` with optional
`destination` and `prehistory` (`low5-prefix`, `low8-prefix`, `zero-until-start`),
or `lzss-12` with `decoded_size` and optional `history`. The encoders avoid
prehistory dependence. Independent game-output witnesses remain external.

`lzss_12_ext::decode(src, decoded_size, max_output)` takes the size from its
caller and reads no carrier header. Zero requested bytes return empty output
without consuming input; this is an API convention, not a carrier zero-count
rule. Matches clamp to the remaining declared size after consuming their full
token and any extension. `decode_with_history` accepts only explicitly known
preceding bytes; no initialized zero prefix is assumed. The existing output
bound applies before allocation. There is no encoder, catalog probe, asset-map
codec selection or automatic detection for this module.

Standalone `swankit-codec-def/1` definitions may explicitly select
`lzss-12-ext` with `decoded_size` and optional known `history`. Its catalog
identity advertises decoding only; the separate external corpus witness checks
complete decoded bytes and consumption, without encoding or round-trip claims.

The same standalone interface accepts `flag16-distance11`. Its header supplies
the output count, so `decoded_size` is optional and, when present, must agree
with that count. Mode zero is a literal copy; nonzero mode selects compressed
commands. Flag words are little-endian and consumed least-significant bit
first; clear bits are literals and set bits are matches. A match token stores
distance minus one in its low eleven bits and length minus three in its high
five bits. Copies proceed forward and can overlap. The source completes each
match before checking its 16-bit remaining count again. The bounded decoder
preserves that behavior and applies `max_output`; it does not clamp a final
match. Explicit history is supported up to the grammar's 2048-byte distance
range, with no implicit fill. The codec is decode-only and has no probe,
encoder, asset-map selection, or default selection.

The standalone definition also accepts `mask16-fill`. Its nonzero count byte
specifies groups of sixteen output bytes; each group supplies a fresh fill byte
and a little-endian 16-bit mask. The low mask byte is tested from `0x80` to
`0x01`, followed by the high byte in the same order. Set bits emit the fill
byte and clear bits consume one literal. If `decoded_size` is supplied, it
must equal sixteen times the count. A zero count is rejected: the source's
16-bit loop underflows rather than describing an empty stream. The bounded
decoder refuses it before allocating output or reading group payload. This
codec has no history, encoder, probe, asset-map selection or default selection.

The standalone definition also accepts `lz-command16-lsb`. Its BE16 header is
the number of commands, not the decoded byte count. Flags are consumed lazily,
least-significant bit first: zero copies one literal byte; one followed by zero
reads a run-length byte and copies that value plus eleven literal bytes; two
one bits read a little-endian token. The token's low twelve bits encode
distance minus one and its high four bits encode length. Nonzero length codes
add two; zero reads an extension byte and adds eighteen. Copies proceed
forward and may overlap. The decoder reports the exact consumed extent and
applies the existing 65,536-byte output limit. Explicit known history can be
passed through the existing definition field; missing history is an error.
This codec has no encoder, probe, automatic selection, asset-map selection or
default selection.
