# Experimental interop boundary

This private prototype compares Java access to the existing `ws-rom` and
`ws-gfx` implementations. It adds no reader or hardware grammar. Hardware
interpretation follows those crates and [hardware sources](../../docs/hardware.md).
The extension's shipping implementation remains independent pending evaluation.

Build `cargo build --release --locked -p swankit-ffi -p swankit-cli` with the
existing pinned toolchain. The cdylib exports the declarations in `swankit.h`.
Check `swankit_abi_version() == 1` before using `swankit_decode_v1`. Callers own
both buffers, provide valid live pointers, and serialize access to their buffers.
No pointer is retained. Success returns the output length; negative statuses are
arguments (-1), parsing (-2), capacity (-3), or a caught native Rust panic (-4).
Errors leave the output untouched. Invalid pointers and native faults cannot be
caught by Rust's panic boundary and can terminate the JVM.

| Operation | Input | Output |
| --- | --- | --- |
| 1 | Whole cartridge, at most 64 MiB; ws-rom validates minimum size and reset opcode | 18 little-endian u32 facts, 72 bytes |
| 2 | 512-byte RGB444 palette region followed by 1..=1024 planar 4bpp tiles, 32 bytes each | Tile order, row-major RGBA8, 256 bytes per tile |
| 3 | Same palette region followed by packed 4bpp tiles | Same RGBA8 layout |

Header words are reset offset, reset segment, maintenance, publisher ID, color,
game ID, version, ROM size code, save code, flags, mapper code, mapper-is-RTC,
stored checksum, computed checksum, checksum-valid, declared ROM size,
hardware-masked reset address, and input size. Unknown declared ROM sizes use
u32::MAX. Tile rendering uses palette zero and opaque alpha; reserved upper
palette bits follow ws-gfx policy. Output capacity is capped at 262,144 bytes.

The experimental CLI offers the same operations over bounded JSON lines:

```sh
swankit interop --json --server
```

Each request is one line with `version: 1`, numeric `operation`, and `path` to
an input file. Each reply has `version: 1` and either an `output` byte array or
an `error` string. Unknown request fields are rejected. Without `--server`,
the process handles one request; server mode handles requests until EOF.
Frames are capped at 16,384 bytes before parsing. An oversized request gets
one error and closes the stream; other invalid requests permit recovery.
Input file metadata and actual reads are bounded. Clients must impose timeouts
and response bounds, serialize requests, and close/reap their own child.
File loading and decimal-byte JSON are measured costs of this prototype.

For `wasm32-unknown-unknown`, additional `swankit_alloc_v1(length)` and
`swankit_free_v1(pointer, length)` exports manage buffers in linear memory.
Free requires the original live pointer and allocation length exactly once.
Wasm traps are runtime errors, including aborting Rust panics. The Java adapter
uses a separate instance per caller and reuses source/output buffers. Build
the target in a container; no host target installation is required.

Independent Java oracles are enrolled outside Git with
`tools/stage-corpus.py --interop-manifest "$ORACLE_MANIFEST" --output "$STAGING_DIR"`.
Enrollment refuses to overwrite a frozen selection. Validate with the
Formatkit safe runner, explicit root and required mode. Optional live Java
commands are supplied through external `SWANKIT_INTEROP_PARITY_CONFIG`; each
command is an argv array. `SWANKIT_INTEROP_BOOTSTRAP=1` permits live parity
before the fresh selection is enrolled, and must be unset for final validation.
