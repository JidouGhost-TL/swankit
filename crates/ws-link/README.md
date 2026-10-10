# ws-link

Ordered, bounded WonderSwan serial byte relay for Linux. It joins two byte
streams through TCP, a loopback TCP listener, stdin/stdout, a shell child,
a raw serial device or the existing `ws-wondergate` adapter core.

```rust,no_run
use std::sync::atomic::AtomicBool;
use ws_link::{Endpoint, Options, Relay};

let endpoints = ["listen:3000".parse::<Endpoint>()?, "exec:cat".parse()?];
let relay = Relay::new(endpoints, Options::default(), Box::new(std::io::sink()))?;
let stats = relay.run(&AtomicBool::new(false))?;
# Ok::<(), std::io::Error>(())
```

Neither end is read until both are connected, including during reconnect.
Early data waits in OS buffers. Each direction has a 65,536-byte pending
limit; full queues apply backpressure. Already-read pending bytes are dropped
and logged on disconnect. Unread data on the retained end waits for its peer.
TCP connections retry every second; listeners accept one peer at a time.
Fixed endpoints cannot reconnect. Cancellation closes owned endpoints and
terminates child process groups.

`Options` supplies latency, seeded jitter and device baud. Scheduling preserves
order independently in each direction. TSV byte records contain direction,
microseconds since relay start, a two-digit hex value, and forwarding/drop
status. Connection events are comment lines. Timestamps describe forwarding
or dropping, rather than serial arrival or hardware timing.

The stream carries data bytes only: no speed, clock, framing errors or control
registers. Endpoints own pacing and receive-buffer behavior. Serial devices use
Linux termios2, raw 8N1, no flow control, and accept 9600, 38400 and 192000 baud.
PTY tests cover those settings; real console hardware remains untested.

See [link recipes and interoperability notes](../../docs/link.md), including
the stock Mednafen helper test and its limits.

External recorded payloads can be enrolled using `tools/stage-corpus.py
--link-manifest "$LINK_MANIFEST" --output "$STAGING_DIR"`. The manifest supplies
input hashes, independently expected output hashes/lengths, direction, delay
settings and oracle provenance. Enrollment creates a new frozen `ws-link`
selection without replacing any existing spec. The dev-only Formatkit corpus
handler sends these payloads through the actual relay and checks delivered
bytes and drops. Run through the serialized safe runner with an explicit root
and required mode; do not regenerate expectations.
