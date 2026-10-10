# ws-wondergate

A bounded, sans-IO WonderGate device core. Network operations use injected
`Network`/`Connection` traits; `backend::Scripted` supplies deterministic peers,
fragments, EOF, errors and partial writes without opening host sockets.

`Adapter::push` accepts arbitrary input fragments and appends only command
replies. `try_handle` returns checked errors for malformed known commands;
`handle` supplies the documented wire failure shapes. `finish` reports a
truncated frame on input EOF. Snapshots and events omit PPP credential bytes.
`Config` holds typed settings; INI, redirects and live backends come later.

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
