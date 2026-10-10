# ws-wondergate

A bounded, sans-IO WonderGate device core. Network operations use injected
`Network`/`Connection` traits; `backend::Scripted` supplies deterministic peers,
fragments, EOF, errors and partial writes without opening host sockets.

`Adapter::push` accepts arbitrary input fragments and appends only command
replies. `try_handle` returns checked errors for malformed known commands;
`handle` supplies the documented wire failure shapes. `finish` reports a
truncated frame on input EOF. Snapshots and events omit PPP credential bytes.
`Config::from_ini` reads the legacy sections and `[swankit]` settings. The
`backend::Tcp` resolver denies unlisted names by default; exact mappings,
explicit hostname redirects, a catch-all mapping or system-DNS opt-in
control resolution. Connect, receive and send waits have timeouts.

The built-in BPL responder is enabled by default and handles fragmented and
consecutive bounded messages entirely in memory, preserving header bytes 8..23
in its 24-byte allow reply. Ordered hostname/socket redirects follow the C
matching rules. `backend::Transcript` checks every request byte against
external per-connection recordings and supplies bounded response reads.
`transcript::{parse, read, frames, Tap}` reads and writes serial tap logs.

```rust
use ws_wondergate::{Adapter, Config, backend::Scripted};
let mut adapter = Adapter::new(Config::default(), Scripted::default());
let mut reply = Vec::new();
adapter.push(&[1, 0, 1, 2], &mut reply, &mut |_| {});
assert_eq!(reply, [1, 0, 3, 2, b'1', b'0']);
```

See [protocol policy](../../docs/wondergate.md) for both profiles, limits,
command shapes and the distinction between client evidence and hardware truth.
Ported sources carry MIT notices; other files use the workspace license.
Publication is disabled. Tests use synthetic bytes. Recorded transcripts and
independent expectations stay external for the later adapter corpus milestone.

The optional external differential gate accepts an independently compiled
WonderFence binary through `WG_C_ADAPTER`:

```sh
WG_C_ADAPTER="$UNPATCHED_ADAPTER" tools/wg-differential.sh upstream
WG_C_ADAPTER="$PATCHED_ADAPTER" tools/wg-differential.sh revival
```

Each invocation compares 10,000 seeded streams over owned loopback peers. The
second C binary needs the original harness patch plus corrections 01–07.
Details, safe-input exclusions and the INI reference are in the protocol doc.
