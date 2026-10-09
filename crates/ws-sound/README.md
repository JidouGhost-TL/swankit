# ws-sound

WonderSwan wave RAM, sound registers and decoded driver records, read through
Formatkit's checked readers and fields. PCM uses `formatkit_core::Pcm`. This
private crate inherits the workspace publication policy.

| API | Input and behavior |
| --- | --- |
| `Wave::decode`, `WaveBank::from_ram` | 16-byte, 32-sample waves; four channels in a 64-byte RAM block at `$8F << 6`; low nibble first |
| `instrument::wave_bank` | A bounded bank of 16-byte waves |
| `PitchEnvelope`, `VolumeEnvelope` | Byte-driver delay/delta and state/table records, with explicit step limits |
| `hardware::Registers`, `Noise` | Divisor, L/R volume, sweep/noise/control and voice register policy; eight LFSR tap modes |
| `hardware::HyperVoice` | Documented sample extension, shift, rate and mono/stereo input routing; direct signed outputs |
| `drivers::RomView` | Explicit bank state, using ws-rom's padded-chip mapping |
| `drivers::byte` | Distinct A/B/C headers and command lengths, two-group varlens, direct mode, PCM SE commands and mask-zero C PCM descriptors |
| `drivers::cell` | Four-byte directory groups, 13-byte song records, PCM descriptors and four-byte track cells |
| `drivers::sdk` | Standard six-byte track and eight-byte voice records, two-byte cells and linked pitch lookup |
| `render::render` | Ordered frame events to 24 kHz signed stereo PCM |
| `drivers::sdk::render_sequence` | A bounded single-track preview of notes, rests, volume, wave selection, A7 waits and reserved cells |
| `render::unsigned_pcm` | Unsigned 8-bit samples to signed PCM at an explicitly supplied source rate |

```rust
use ws_sound::Wave;

let packed: Vec<u8> = (0..16).map(|n| (n << 4) | (15 - n)).collect();
let pcm = Wave::decode(&packed)?.to_pcm(24_000)?;
assert_eq!(pcm.frame_count(), 32);
# Ok::<(), formatkit_core::Error>(())
```

Record locations, table counts, bank state, wave counts and tick duration are
caller inputs. The crate performs no title detection. Untagged waves and driver
bytes have explicit catalog identities and require a selected grammar; arbitrary
ROM bytes cannot safely identify these formats by magic. Music/SFX classification
is caller policy. Header mask and priority remain available for that policy.
Byte B's high-to-low mask order and reversed slot/channel order are distinct from
A/C. Mask-zero C entries use `ByteCPcm`, rather than the music header parser.
Instruction decoding consumes one token and retains flow operands; the caller
owns track modes, calls, loops and bank changes.

The byte grammar follows the verified `sound-byte` library interfaces and the
owner-maintained sound format study. A PCM skip on a channel other than channel
2, or with call depth at least six bytes, consumes eight operand bytes; the play
path consumes nine. A direct sweep token uses its own opcode as the amount and
its operand as the tick period. C's terminating commands and PCM no-op differ
from A. Raw command operands remain visible where effect state is incomplete.
The SDK grammar is the standard two-byte-cell family. Compact variants require
separate evidence and are not silently interpreted as standard cells.

The hardware sources and reference disagreements are recorded in
[hardware.md](../../docs/hardware.md). The owner-authored reconstruction evidence
is in the sibling SDK research workspace's `libs/sound-byte`,
`libs/sound-sdk/source/engine-c016` and `docs/sound-opcode-corrections.json`.
These are format evidence, not hardware documentation. No emulator implementation
code is used.

Rendering is a preview with documented limits. The default uses integer phase
accumulation, samples once every 128 CPU cycles and applies events before the
named output frame. Wave index starts at zero as a preview convention; enabling
a channel holds/resumes its phase and does not reset its index. Divisor changes
act on the next frame's phase calculation. Sweep uses 375 Hz ticks; writing
`$8D` resets the sweep step count. Noise clocks in wave mode too. Channel 2 voice
needs bit 5, without requiring bit 1. Headphone output needs `$91` bit 3; mixing
adds unsigned channel values, shifts by five and wraps additions with Hyper
Voice to 16 bits. `render_coupled` uses the Java listening preview's floating
phase and one-pole DC filter (coefficient 0.0005). Its occasional floating phase
edge differs from integer phase; the filter is not a hardware circuit model.

The renderer does not execute driver code, resolve full call/loop/envelope
state, emulate sound DMA or an IRQ sample pump, implement the speaker path or
sound test ports, or reconstruct CPU-cycle write timing. Supply PCM and Hyper
Voice input updates explicitly as events. Hyper Voice rate is descriptive;
sample arrival timing is caller-owned. Extension mode 3 has unspecified upper
bits and returns `Unsupported`. The SDK sequence preview rejects effects and
control flow requiring linked state, including sweep, envelopes, slide, loops
and voice. Zero-duration notes/rests occupy one driver tick.

Output is bounded to ten minutes at 24 kHz, one million events and 65,536
records/steps per bounded table. Extents and counts are checked before parsing
or allocating. The SDK renderer also requires a caller output bound; timeline
overflow returns an error. Truncated operands, unmapped or wrapping pointers,
unknown opcodes and unsupported effects return errors. Parsing a record consumes
its fixed prefix; callers may retain a surrounding stream or table.

Synthetic tests cover hardware policy, distinct grammars, malformed input and
resource bounds. Dev-only Formatkit corpus handlers compare external independent
table/cell digests, unsigned PCM exports, instrument records, synthetic Java
previews and existing register-capture WAV excerpts. Spec populations and golden
digests stay external; ordinary tests use no configured corpus. The staging tool
adds a new sound selection and refuses to overwrite any frozen selection.
