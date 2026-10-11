# ws-wondergate-server

Private, newly authored WonderGate services and bounded title definitions. The core performs no I/O
or outbound networking. HTTP parsing retains the raw request and binary body;
text fields map each Latin-1 byte to the same Unicode code point. Response
serialization fixes Content-Length, Connection, Content-Type and extra-field
order, without automatic Date or Server fields.

`Server::bundled` takes a `Store`, `Clock` and `Rng`. `dispatch_http` handles the
four charge endpoints, free billing and bundled data-defined routes;
unrecognized routes return 404.
`session(80)` and `session(5555)` provide bounded sans-I/O HTTP and BPL sessions.
HTTP closes after one transaction. Consecutive BPL messages each receive a
24-byte field-preserving allow reply. A feed batch is capped at 64 KiB.
Service identities and the allowed absolute-URL alias pair live exclusively in
[services.json](data/services.json), validated when loaded. Title routing,
templates, checked fields, phase values, checksum envelopes and ranking actions
are interpreted from embedded JSON. See the [read/write guide and plugin
ledger](../../docs/wondergate-title-definitions.md). `Server::from_definitions`
composes validated custom programs; unknown plugins fail to load.

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
the journal. An append failure disables further writes until reopening. Capacity refusals do
not poison the store. Both stores refuse new records at 1,024 tokens total,
256 per host, 4,096 submissions total, 2,048 per board and 64 per identity
within a board. Existing tokens remain reusable. The journal additionally
refuses growth beyond 16 MiB, including at reopen. Refused HTTP writes return
500 without adding history; a full store must be archived and a fresh store
chosen explicitly. Records are never evicted or silently lost.
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
HTTP and BPL have separate connection pools, with at most four active
connections per peer address across both. Excess connections are closed. BPL
closes after 32 messages or four times the configured timeout in total; idle
reads retain the configured timeout. Deadline and connection-cap refusals are
normal shutdowns. Shutdown joins active workers after their socket timeout. Wire files are
exclusive-created, and JSON entries use the Python `wire.jsonl` field names,
lengths, SHA-256, UTC time, peer array and optional error. Restart never replaces
existing request or response bytes. Wire logging refuses further records at
256 transactions or a conservative 16 MiB cumulative reservation (raw request
plus response plus 4 KiB metadata per transaction). Counts include existing
files on reopen. A single stderr diagnostic reports refusal; serving continues
without further log growth, and shutdown remains successful. Actual filesystem
failures remain worker errors. The bundled human-readable ranking pages show
at most 256 rows to fit their 64 KiB body limit. The game ranking response
retains its three leaders, submitter and fourth-place records.

The complete ported service suite and additional synthetic schema, field, template, parser,
journal, ranking and loopback tests run in ordinary CI. Independent Python
HTTP responses and PHASE0-corrected BPL expectations remain external:

```sh
python3 tools/stage-corpus.py --wondergate-server-manifest "$ORACLE_MANIFEST" \
  --output "$STAGING_DIR"
FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus
```

Enrollment validates both independent request/reply digests and freezes a new
`ws-wondergate-services` selection for shared vectors, or a separate
`ws-wondergate-server` selection for native HTTP capture manifests with explicit
empty starting stores. The latter compares full reply bytes and frozen hashes.
Existing selections cannot be overwritten.
Protocol evidence and BPL client addresses are in
[WonderGate policy](../../docs/wondergate.md). Ported files are MIT-marked;
new infrastructure inherits the workspace license. See [NOTICE](../../NOTICE).
