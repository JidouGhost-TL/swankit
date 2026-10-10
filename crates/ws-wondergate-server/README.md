# ws-wondergate-server

Private, newly authored WonderGate shared services. The core performs no I/O
or outbound networking. HTTP parsing retains the raw request and binary body;
text fields map each Latin-1 byte to the same Unicode code point. Response
serialization fixes Content-Length, Connection, Content-Type and extra-field
order, without automatic Date or Server fields.

`Server::bundled` takes a `Store`, `Clock` and `Rng`. `dispatch_http` handles the
four charge endpoints and free billing; unrecognized routes return 404.
`session(80)` and `session(5555)` provide bounded sans-I/O HTTP and BPL sessions.
HTTP closes after one transaction. Consecutive BPL messages each receive a
24-byte field-preserving allow reply. A feed batch is capped at 64 KiB.
Service identities and the allowed absolute-URL alias pair live exclusively in
[services.json](data/services.json), validated when loaded. Title routing,
templates and plugins are the next definition-engine milestone.

```rust
use ws_wondergate_server::{FixedClock, SeededRng, Server, Timestamp};
use ws_wondergate_server::store::MemoryStore;

let server = Server::bundled(
    Box::new(MemoryStore::new()),
    Box::new(FixedClock(Timestamp::parse_fixed("20261005200403")?)),
    Box::new(SeededRng::new(123)),
)?;
let mut session = server.session(80);
# Ok::<(), formatkit_core::Error>(())
```

`MemoryStore` starts empty. `JournalStore::open` locks an append-only JSON-lines
file, rebuilds scoped indexes, and fsyncs each new submission or token before
returning success. Every record has `provenance: "new"` and a monotonic id.
Malformed, oversized, non-new or unterminated records fail without rewriting
the journal. An append failure disables further writes until reopening.
Tokens remain stable for `(host, sid)` across retries and restarts. Binary
identities and names serialize as byte arrays. No SQLite migration is provided.

`Board` offers count, bounded leaders, one-based position (zero if absent) and
slot padding. Histories are scoped by definition, board and course. It selects
the best score per raw identity in either order and retains that score's name
and extra fields. An empty identity is a real player. Global ties default to the
identity's first submission, matching the original store. Explicit
`Ties::FirstBestSubmission` matches the later history helper. Equal best scores
retain the first winning record in both modes; empty slots use `None`.

`Clock` supplies checked UTC timestamps; gettime converts them to JST.
`FixedClock` and `SeededRng` provide reproducibility. `Rng::fill` returns errors
so an entropy failure becomes HTTP 500. `OsRng` uses the OS entropy device on
Unix; other platforms require an injected implementation. Seeded entropy is
for tests and deliberate deterministic runs.

Feature `net` adds threaded inbound TCP listeners, default five-second socket
timeouts, a bounded connection count, owned-thread shutdown and `WireLog`.
Listeners default to loopback; bind addresses are numeric and never resolved.
An overall request deadline also prevents trickled bytes from extending a
transaction indefinitely. HTTP allows a 50 ms window for a separately segmented trailing NUL on an empty
body. The sans-I/O assembler allows callers to choose that window themselves.
Shutdown joins active workers after their socket timeout. Wire files are
exclusive-created, and JSON entries use the Python `wire.jsonl` field names,
lengths, SHA-256, UTC time, peer array and optional error. Restart never replaces
existing request or response bytes.

The ported shared/framing service cases and additional synthetic parser,
journal, ranking and loopback tests run in ordinary CI. Independent Python
HTTP responses and PHASE0-corrected BPL expectations remain external:

```sh
python3 tools/stage-corpus.py --wondergate-server-manifest "$ORACLE_MANIFEST" \
  --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

Enrollment validates both independent request/reply digests and freezes a new
`ws-wondergate-services` selection. Existing selections cannot be overwritten.
Protocol evidence and BPL client addresses are in
[WonderGate policy](../../docs/wondergate.md). Ported files are MIT-marked;
new infrastructure inherits the workspace license. See [NOTICE](../../NOTICE).
