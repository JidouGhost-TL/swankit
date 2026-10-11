# WonderGate title definitions v1

The server reads `swankit-wondergate-title/1` JSON programs from
`crates/ws-wondergate-server/data/`. Build-time discovery embeds every JSON
file except `services.json`, in filename order. Title identities, hostnames,
literal routes and title text live in data; Rust code and examples use generic
client lineages. Shared BPL and charge services retain their own service data.

Every definition requires `provenance.server = "new"` and nonempty notes.
Rankings start empty and record new submissions. Static selections and phase
seeds are revival policy. An unimplemented content route can return 501 with a
client-compatible error envelope. These definitions contain no recovered server
history, original downloads or ROM bytes.

Load JSON through `Definition::from_json`. It applies the
[closed JSON Schema](wondergate-title-definition.schema.json), then checks
identities, references, route overlap, field types, header shapes and minimum
response sizes. Unknown properties, primitives and plugins fail to load.
`Definition` is immutable; edit its JSON copy and validate a new instance.

## Reading the format

| Field | Meaning |
| --- | --- |
| `schema`, `id` | Exact version and a stable scope identifier. Identifiers have 1..128 ASCII letters, digits, hyphen, underscore or dot. |
| `provenance` | Required `server: "new"` and `notes` explaining evidence and policy. |
| `hosts` | Nonempty unique normalized hostnames, without port or trailing dot. Definitions may share a host when their paths are disjoint. A path belongs to one definition; shared charge paths cannot be overridden. |
| `boards` | Named ranking policies: `identity` field, `order: "asc"` or `"desc"`, `ties`, and explicit `course`. Storage scope is `(definition id, board id, course)`. |
| `values` | Named `cyclic-phase` values, described below. |
| `records` | Named segment arrays for a row. A record cannot contain another `each` or `row`; this prevents recursive templates. |
| `routes` | Nonempty ordered route declarations with unique ids, method, selector, fields, actions and response. Overlapping selectors for the same path and method fail to load. |
| `limits` | Required `max_body` and `max_response`, in bytes, including envelopes and all HTTP bytes respectively. |
| `errors` | Optional default `host_other_path`, `method` and `invalid` status codes (400/404/405), plus `method_first`. |

A route has `id`, `method` (`GET`/`POST`/`HEAD`), `match` and `respond`.
An explicit `HEAD` route renders the declared body to determine Content-Length,
then sends headers only. HEAD routes cannot mutate boards.
Optional `bare_get: true` accepts exactly `GET <declared-path>\r\n\r\n`
and sends the rendered body without HTTP headers. It requires a static GET
route with one host, no query selectors, request fields or actions. Missing or
ambiguous declarations reject the request; strict HTTP parsing remains the
default. Received wire bytes are preserved.
`match.path` is an ASCII origin path. Optional `query_exact` compares the raw
query without percent decoding; optional `query_keys` matches the exact set of
field keys. These two selectors are mutually exclusive. Without either, the
route accepts any query. A field selector uses query bytes by default;
`source: "form"` selects the body and requires POST plus a content type of
`application/x-www-form-urlencoded` (parameters are accepted).

Origin paths retain repeated slashes, including an initial `//`; the Host
header supplies their authority. An explicit `http:` URI supplies a separate
authority and remains subject to the configured host-alias check. This follows
[RFC 9110 section 4.1](https://www.rfc-editor.org/rfc/rfc9110.html#section-4.1)
and [RFC 9112 section 3.2.1](https://www.rfc-editor.org/rfc/rfc9112.html#section-3.2.1).

`request` declares every selected field. Missing or additional keys fail.
Field names are literal ASCII identifiers and are never percent decoded.
Duplicates fail by default (`repeats: "one"`). Explicit `"first"`/`"last"`
chooses an occurrence, but all occurrences must pass validation.

| Field option | Interpretation |
| --- | --- |
| `decode` | `"raw"` (default) or `"percent-plus"`: plus becomes space and `%HH` becomes one byte; incomplete or nonhex escapes fail. |
| `bytes` | Inclusive `[minimum, maximum]` decoded byte lengths; default `[0,512]`. |
| `equals`, `deny`, `min_byte` | Exact decoded bytes, disallowed byte values, and lower bound for every byte. |
| `charset` | `"raw"` (default), `"ascii"`, or strict standard `"shift-jis"`. Length always counts bytes. |
| `regex` | Only anchored decimal widths: `^[0-9]{N}$` or `^[0-9]{N,M}$`, with 1..20 digits. Other regex grammars are unsupported. |
| `as` | Checked unsigned `"u32"` or `"u64"`; signs, nondigits and overflow fail. Without a regex, numeric width is 1..20. |
| `optional`, `default` | Permit an absent field, or supply default bytes and validate them. Action fields must always be available. |
| `choices`, `alphabet` | Allowed complete byte values, or an ASCII alphabet for each decoded byte. |
| `prefix`, `suffix`, `last_bytes` | Required byte prefixes/suffixes, or allowed final bytes; checked before trimming. |
| `trim_end`, `max_number` | Remove trailing occurrences of one byte after validation; cap a declared numeric value. |

`tables` declares repeated delimited fields separately from `request`. Each
table has a nonzero byte `delimiter`, `optional`, and `rows` mapping a tag to
one through eight checked column declarations. Values are percent-plus decoded
before splitting. A row must have exactly the declared columns. Optional
`tag_decimal_width: [minimum, maximum]` parses a checked decimal tag and
normalizes it to its canonical decimal key; aliases such as `01` and `1`
therefore count as a duplicate. Without this option, tags match literal bytes.
All tags and columns in every table are validated before any submission.

`table_actions` maps a table `field` and its tags to `boards`, with required
request `identity` and `name` references, a numeric `score` column index and
optional numeric `extra` column index. The latter must have a declared maximum
inside the signed 64-bit range. Optional route `form_trim_end` lists tolerated
trailing form bytes; this is applied before form splitting. Neither trimming
nor numeric tag normalization is implicit.

A route may have one action: `record` or `increment`, with `board`, `identity`,
`name` and numeric `score` field references. The identity reference must match
the board's identity declaration. All fields are validated before writing.
`record` appends a submission; `increment` adds to the identity's previous best
score with checked arithmetic. The server serializes dispatch, including this
read/add/write sequence. A submission carries the injected UTC timestamp and
journal provenance `new`.

Boards select the best score per raw identity. Equal best scores keep the first
winning record and its name. `ties: "first-submission"` compares identities by
their first submission; `"first-best-submission"` compares their first winning
submission. Leaders follow the chosen score order, and position is one-based.
Padding emits empty rows; it never creates journal submissions. The submitter
row uses the current request's name/score and its position in the best-score
board, even if this request did not improve its score.

Unrecognized hosts return 404. For a known host, an unmatched path uses
`host_other_path` (default 404), a wrong method uses `method` (default 405), and
an unmatched query or invalid field uses `invalid` (default 400). Route errors
override definition defaults. By default selectors run before the method
check. Definition-level `method_first: true` checks host methods before paths;
route-level `method_first: true` checks methods before that path's selectors.
This preserves the original handlers' different error precedence.

## Building response bytes

`respond` has `status` (200/400/404/405/500/501), `body`, optional
`content_type`, ordered `headers` as `[name,value]` pairs, and `expect_lines`.
Optional `keep_alive: true` permits sequential HTTP requests on one socket.
The default emits `Connection: close`. An explicit client `Connection: close`
overrides the definition. TCP reuse is bounded to 128 transactions, each with
the configured deadline and input limits; pipelined requests are rejected.
The serializer emits status, Content-Length, Connection, optional Content-Type,
then extra headers and the blank separator. It adds no Date or Server header.
Header names must be unique, including Content-Type; Content-Length and
Connection are reserved. CR/LF injection and non-ASCII headers fail to load.
`expect_lines` counts every CRLF, including the blank separator. The VT-lineage
phase definition uses seven lines and mirrors its three state bytes in its
11-byte body, keeping the complete response inside its client buffer.

An authored local browser page uses a literal `text` body with
`content_type: "text/html"` and explicit body and complete-response byte limits.
Its definition records the observed home route and labels the page as new
revival content. Browser rendering requires a native display check in addition
to HTTP status and byte tests.

`body` is a segment array, or an object with `envelope` and `segments`.

| Segment | Output |
| --- | --- |
| `{"text":"..."}` | UTF-8 bytes, without a terminator. |
| `{"bytes":[0,255]}` | Exact byte integers. |
| `{"zero":N}` | N zero bytes. |
| `{"value":"phase.mode","as":"u8"}` | A named phase component: `mode`, `minute`, `second` or `elapsed_s`. Numeric formats only. |
| `{"field":"score","fmt":"dec"}` | Row `rank`, `name`, `identity`, `score` or `extra`. Only inside a record. |
| `{"each":{"leaders":{"board":"main","limit":3,"pad_to":3}},"record":"row"}` | Bounded leaders followed by empty rows until `pad_to`; larger real selections are retained. |
| `{"row":{"submitter":"main"},"record":"row"}` | Current submission; requires an action on this board. |
| `{"row":{"place":{"board":"main","n":4}},"record":"row"}` | One-based placed leader or an empty row. |
| `{"date":14}` | Injected UTC timestamp as 14 decimal digits; `8` emits just the UTC date. |
| `{"board":"main","metric":"position","identity":"owner","fmt":"dec"}` | Board `count`, request identity's `position`, or selected `length`. Length uses bounded `limit`/`pad_to`; `max` clamps, or `overflow_zero` emits zero beyond it. |
| `{"when":{"equals":{"mode":[49]},"absent":["list"]},"segments":[...]}` | Render only when every declared byte equality and absence condition holds. Conditions refer to validated request fields, including defaults. |
| `{"when":{"submitted":["main"]},"segments":[...]}` | Render only when the current request submitted to every named board. Prior journal entries do not satisfy this condition. Board references are validated at load time. |

Numeric formats are `u8` (one checked byte), `dec`, `hex-lower` and `hex-upper`.
Row numeric options `width` and `zero_pad` control minimum text width/padding;
`min`/`max` clamp the value; `empty` replaces a padded row's value. Options for
text padding do not apply to `u8`. Text width does not truncate a larger number.
Negative `extra` cannot be rendered as unsigned. Byte fields use `raw` or
`html-shift-jis`, without numeric options. The latter decodes standard Shift-JIS
with replacement for invalid bytes, escapes `&<>"'`, and emits UTF-8.
Windows-only characters remain invalid, matching the Python oracle's codec.
Raw byte fields can declare a minimum `width` with space padding and
`empty_bytes` for a structural empty row. Numeric row `divide` is a nonzero
unsigned divisor applied before formatting and clamping. These operations never
truncate names or turn structural rows into players.

The envelope is `primitive: "sum16-hex-envelope"`, an ASCII `marker`,
`case: "lower"`/`"upper"`, and literal byte `terminator`. Sum every payload
byte modulo 65536; render four hex digits. Output is
`marker + checksum + payload + marker + checksum + terminator`.
The sum excludes both markers, both checksums and the terminator. The bank13
lineage pads three leaders, emits the submitter then fourth place, and wraps
the complete record payload once.

A `cyclic-phase` value declares `period_s` (1..15360), unique byte `phases`,
and `policy`. `"fixed"` requires `fixed.phase` from that list and
`fixed.elapsed_s < period_s`. `"clock"` forbids `fixed` and uses the injected
UTC seconds with optional `epoch_s` (default zero). Euclidean division chooses
`phases[floor((now-epoch)/period) modulo phase_count]`; Euclidean remainder
is elapsed seconds. Minute and second are elapsed/60 and elapsed modulo 60.
Negative offsets wrap predictably. No sleeping or system-clock access occurs
inside the interpreter.
One injected timestamp supplies every phase component and action in a title
transaction, so a clock rollover cannot mix values within one reply.

## Read and write a definition with Rust

```rust,no_run
use serde_json::json;
use ws_wondergate_server::{Definition, FixedClock, SeededRng, Server, Timestamp};
use ws_wondergate_server::store::MemoryStore;

fn edit_and_run(text: &str, request: &[u8])
    -> formatkit_core::Result<(String, Vec<u8>)>
{
    let before = Definition::from_json(text)?;
    let mut candidate = before.json().clone();
    candidate["provenance"]["notes"] = json!("Reviewed newly authored policy");
    let after = Definition::from_json(&candidate.to_string())?;
    let reviewed_json = after.to_json()?;
    let server = Server::from_definitions(
        vec![after], Box::new(MemoryStore::new()),
        Box::new(FixedClock(Timestamp::parse_fixed("20261005200403")?)),
        Box::new(SeededRng::new(123)),
    )?;
    Ok((reviewed_json, server.try_dispatch_http(request)?))
}
```

1. Read JSON at the application boundary and load it with `Definition::from_json`.
   `Definition::bundled()` loads embedded programs; `definitions()` exposes
   their original JSON strings. `id()`, `hosts()` and `json()` are read views.
2. Edit a JSON copy, preserving the identity unless a new history scope is
   intended. Validate the candidate and review `to_json()` before saving it.
3. Compose definitions with `Server::from_definitions`. For custom shared
   services and HTTP limits use `Server::with_definitions`; duplicate ids and
   shared-host path collisions fail. `Server::bundled` loads the shipped service
   and title data together.
4. Supply a store, clock and RNG. Use `MemoryStore` for an empty test history,
   or `JournalStore::open` for a durable journal. Render synthetic requests and
   check status, full bytes, header line count and body/client response limits.
5. Save reviewed JSON at the application boundary. The engine opens no
   definition files and does not rewrite its input. The CLI embeds definitions
   at build time; rebuild after editing shipped data. Its `--data` directory
   stores the journal and does not override definitions.

## Read and write from another language

Validate JSON against the schema, then implement the semantic checks above;
schema validation alone cannot prove that references or response bounds work.
Preserve raw HTTP/body bytes and percent-decoded names, and keep explicit UTC
seconds and unsigned integer arithmetic. Apply the field declarations before
any action, retain board scope and tie order, then execute segment arrays in
order. Keep byte length separate from Unicode character count. Serialize HTTP
headers in the stated order and calculate Content-Length after the envelope.

An editor can change policy without implementing the whole engine: parse JSON,
copy it, change only the intended field, validate it with this Rust API, then
save the candidate. Replaying independent frozen requests validates wire
compatibility. Never replace a capture's expected output with newly generated
Rust output.

## Bounds, checks and plugin ledger

Definition text is capped at 256 KiB. The schema bounds routes, hosts, boards,
values and field keys; templates have at most 1024 segments, at most 4096
leaders/padded rows and no recursive row records. Fields are capped at 512
bytes and forms at 64 KiB/128 occurrences. HTTP body and response hard ceilings
are 65536 and 81924 bytes. Declared client limits may be smaller. Static minimum
sizes fail at load time; every append, zero run, numeric conversion and envelope
also checks its runtime bound before allocation or indexing. Rendering/configuration
failures produce 500; malformed client fields produce the declared error.
A journal action is durable before response rendering, so a later size failure
does not undo an accepted submission.

| Registered plugin | Why native logic is required | Consumers |
| --- | --- | --- |
| None | The current client lineages fit bounded data primitives. | None |

There is no registered plugin API in v1. A `plugin` member fails closed. A
future plugin must use a generic identifier, load-time parameter validation,
an explicit registration and a ledger entry explaining why data cannot express
its independent consumers. No dormant native handler is added for future work.

The bundled ranking programs additionally cover nine delimited client grammars:
four-course packed scores, four-course fixed-width single leaders, two groups
of sixteen ascending time boards, five-course scores with decimal conversion,
one-course five-row lists, fourteen-course list/upload selectors, bounded
sixteen-row lists with byte extras, fifteen-course three-slot lists, and a
one-course fifty-row body installer. Their hosts and title identities stay in
JSON data. Empty histories, best-per-identity ordering, UTC dates and structural
padding are newly authored revival policy. Parser acceptance alone does not
establish native connectivity, eligibility, rewards or original server content.

Independent history-backed vectors enroll with the existing
`--wondergate-server-manifest` option and schema
`swankit-wondergate-ranking-manifest/1`. Each HTTP record binds a synthetic
starting history with `store_path`/`store_sha256`, in addition to request and
response paths/digests, fixed clock, seed and oracle provenance. Staging creates
a new `ws-wondergate-rankings` selection and refuses replacement. The required
safe runner checks the history digest, bundled production definitions and full
reply bytes without rewriting any expectation. The generic
`definition_vectors` example accepts external definitions, submissions, raw
requests and a clock as JSON lines for independent oracle comparisons.

Run ordinary checks with corpus variables unset. Schema/semantic/form/primitive
and lineage service tests use synthetic inputs. Independent native HTTP captures
are enrolled once with `tools/stage-corpus.py --wondergate-server-manifest` into
`ws-wondergate-server`; shared vectors use `ws-wondergate-services`. All starting
stores in title capture manifests are explicitly empty. Existing selections
cannot be overwritten. Use `FORMATKIT_CORPUS_DIR="$STAGING_DIR" tools/ci.sh --corpus`
for required serialized validation. Hashes, golden bytes and population counts
remain external. The corpus gate compares complete response bytes as well as
frozen digests. See [protocol evidence](wondergate.md),
[CLI use](cli.md#wondergate-server) and [license notices](../NOTICE).

External oracle drivers and corpus tests keep bounded fixture history immutable,
validate each row through the owning store, and order fresh rows after the
fixture rows. Fresh writes and token issuance use the production store quotas.
This development-only fixture backend does not import historical rows into a
live journal or change the shipped server's cumulative refusal limits.
