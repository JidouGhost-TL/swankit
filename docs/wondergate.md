# WonderGate adapter core

## Raw executables

`cargo build -p swankit-cli --locked --bins` builds `swankit`, `wg-adapter`
and `wg-decode`. `swankit wg-adapter` and `swankit wg-decode` share the same
command entrypoints as the stand-alone executables.

```sh
wg-adapter --config adapter.ini --tap serial.log
wg-decode serial.log
```

Set Mednafen's `wswan.excomm.path` to `wg-adapter`. Mednafen supplies the
executable as argv0 and the literal `ASDF` as its first argument. The executable
ignores that sentinel and reads configuration from `WG_INI`. Outside Mednafen,
`--config` or a positional INI path takes precedence over the environment.
Missing explicit configuration is an error; without a path the revival
defaults apply. Stdout contains only reply bytes, flushed immediately. Stdin
EOF closes the device and exits; an incomplete final frame returns an error.

`--tap FILE` creates a new lossless byte log; it refuses to overwrite a file.
`WG_SERIAL_LOG` instead appends to the harness's precreated log. The format is
`T <milliseconds> > <hex>` for input and `<` for replies. Tap files contain
the original wire bytes, including credentials. Diagnostics stay on stderr.
The raw byte pump and bounded INI loading are shared with `swankit link`'s
WonderGate endpoint. Raw mode currently accepts `--net tcp` only.

The TCP resolver remains denied by default. Use exact mappings or redirects
for local testing; `resolve_all=127.0.0.1` in `[swankit]` directs all names to
loopback inside an isolated harness container. The harness uses
`--network=none` and the existing Python revival server.

`wg-decode` reuses the checked transcript reader and core frame decoder,
preserving directional byte counts, offsets, command names, hexadecimal
parameters and printable ASCII columns. Its default is `--header len16be`;
`--header zero-then-len8` selects the upstream grammar. Zero padding is skipped
at frame boundaries. Invalid records and incomplete frames fail without
emitting a partial listing. Physical UART timing is owned by the emulator.

`ws-wondergate` implements a request/reply device with injected networking.
There is no unsolicited output, host DNS, socket listener, timer or UART in
this core. The traits own their backend wait/timeout policy. Scripted peers
are exact endpoint matches with explicit fragment, EOF, error and partial-write
outcomes. Unknown commands produce no reply.

Protocol references: [WSdev WonderGate](https://ws.nesdev.org/wiki/WonderGate)
and [WSdev BPL](https://ws.nesdev.org/wiki/WonderGate/bplXX.mopera.ne.jp) (CC0).
[UART](https://ws.nesdev.org/wiki/UART) documents the physical serial layer;
this implementation does not simulate it. Hardware behavior remains unmeasured.

Phase-0 client evidence supports a **16-bit big-endian** length after frame
type, counting command plus parameters. F hostname send stores CH/CL, and
F/VT receive byte-swaps the length word. Representative ROM offsets are
F send `39ED39/39ED3C`, receive `39F136/39F13A`, far F send `3A1AF5/3A1AF8`,
receive `3A1F15/3A1F19`, and VT receive `34EAC2/34EAC6`.
R/S/S' outgoing wrappers emit zero plus a low byte; their DNS receivers use
the low byte (S' at `3FBA15/3FBA3F`). S' fixed reply readers use explicit byte counts. Short frames work
with both header modes. These observations establish protocol compatibility,
not physical adapter timing. Detailed excerpts and title attribution belong
in the external job evidence.

The custom browser BPL exchange at `1E8B37/1E8B55` reads exactly 24 bytes
before inspecting `(LE16(header+2) & 003F) == 0010` at `1E8B6C/1E8B70`.
The dispatch at `1E8B8F` returns B024; its caller continues with HTTP. The
eight-byte revival-server reply is incompatible with this exact read.
The declared BPL policy copies the 24-byte request header, writes
`01 18 10 00 18 00 00 00` at offsets 0..7 and preserves 8..23. Preservation
is conservative policy; that allow branch does not compare the metadata.
The built-in responder implements this policy; external backends still move
BPL bytes opaquely when `builtin_bpl` is disabled.

| Profile field | UPSTREAM | REVIVAL (default) |
| --- | --- | --- |
| header | ZeroThenLen8 | Len16Be |
| first_socket | 30 | 00 |
| close_reply | five bytes | six bytes |
| readline | block alias | through LF |
| eof | count 00 | count FF on positive read |
| poweroff_parameter | one zero parameter | none |
| skip_zero_padding | false | true |
| f_stack_metadata | false | true |
| mode_select | false | true (alternate initialization) |
| poll_reports_allocation | false | true |
| dial_modes | 03 | 01, 03 |
| status_params | 03 | 00, 03 |
| status_without_parameter | false | true (parameterless client form) |
| bpl_reply | FieldPreserving24 | FieldPreserving24 |

Each field is individually selectable. Profiles reproduce the pinned MIT
WonderFence command rules and the seven local client-derived corrections.
Small IDs, empty metadata, incoming-call success and allocation-only poll
semantics remain compatibility policy. Legacy poll ignores parameters, matching its C handler. Strict validation of malformed
parameters and checked lengths intentionally replace upstream unsafe reads.
In legacy mode byte 1 is ignored on input, matching C; output writes zero.
The internal C no-reply marker never becomes a length flag.

Select framing independently of the command profile in the adapter INI:

```ini
[swankit]
profile = revival
header = zero-then-len8
```

This keeps revival socket, EOF and padding behavior while using the client
grammar `[type][ignored byte][length8][command][parameters]`. The length
counts the command and parameters. Use this explicit setting only when the
client sender establishes the eight-bit grammar. A zero-length header returns
an error and resets framing; subsequent interframe zero padding is skipped.
For example, a malformed startup prefix `01 8f 00 01 02` then zero padding
can recover at a later valid request in this mode. BE16 instead interprets
`8f 00` as a declared length and drains that oversized body. No startup byte
is silently deleted and BE16 bounds/draining remain unchanged.

The following table lists **reply parameters**, following `[type][BE length]
`[command]`. Results/IDs/counts are byte-sized; IPv4 order is network order.

| Request | Parameters / action | Reply command and parameters |
| --- | --- | --- |
| 01:02 | power on; reset only on off-to-on transition | 02, ASCII `10` |
| 01:00 | check PDC | 00, `[pdc][reception]` |
| 02:01 | status selector from profile, or none when enabled | **02**, `[status]` (off 00, on 02, PPP 03) |
| 02:00 | exactly `03`, revival only | **02**, no parameters; initialize the same local powered-on state as 01:02 |
| 01:01/03/04 | enabled metadata policy | same command, `[00]` |
| 01:08 | `00 01 mode n phone[n]` | **0B**, result 00 or FF |
| 01:09 | enabled answer policy | **0B**, result 00 |
| 01:0A | `01`; hang up | **0B**, result 00 or FF |
| 01:10 | `07 00 00 00 00 01 ulen user plen password` | 10, success 01, malformed 00 |
| 01:11 | `04`, eight prefix bytes, two IPv4 addresses | 11, success 01, malformed 00 |
| 0F:FF | `55 AA 55 AA` confirmation | FF, no parameters (legacy `[00]`); bad confirmation silent |
| 11:01 | `00`; reserve ID without backend I/O | 01, ID or FF (table full) |
| 11:03 | ID, BE16 family 0/2, LE16 port, IPv4, eight reserved bytes | 03, `00 success` |
| 11:06 | ID, mode | 06, `00 allocated` (legacy `[01]`) |
| 11:07 | ID; close and release | 07, `00 existed` (legacy `[existed]`) |
| 11:08 | bounded NUL-terminated hostname | 08, `01 name NUL IPv4`, or `[00]`; zero IPv4 fails |
| 11:0E | ID plus data | 0E, `00 written`, or `00 FF` |
| 11:0D | ID, requested byte count (optional trailing byte) | 0D, `00 count data`; stops after LF in revival |
| 11:0F | ID, requested byte count (optional trailing byte) | 0F, `00 count data` |

`try_handle` returns Formatkit errors before mutating state for malformed
commands. `handle` and `push` use the table's failure forms; commands with no
failure encoding are silent. `push` additionally emits `Dropped(error)`.
Invalid or unconnected read IDs give `00 FF` with no payload. Zero-size reads
on connected sockets consume nothing, even after EOF. Line tails preceding
EOF/error are returned first, then the terminal result. Block reads retain
backend fragment boundaries. Close releases an ID for reuse; adapter drop or
the next power-on reset closes remaining connections exactly once. Poweroff
itself only changes power status, matching the upstream state model.

Frame lengths are checked before allocation. `Frame::encode` returns a Result
and refuses an unrepresentable length. `Limits::max_frame` counts command and
parameters, defaults to 4096, and is capped by the selected header mode.
`max_sockets` defaults to 255; ID FF is always reserved and the table is fixed.
Read data is capped at 254, or 252 in legacy framing, and further bounded by
the reply limit; FF remains an unambiguous error/EOF count. A positive read
with no room for any reply data fails before backend I/O. Oversized writes
that cannot fit a non-FF count return an error rather than truncate the count.
DNS success size is bounded before resolving or allocating its reply.
Out-of-contract backend send/receive counts give FF and never copy excess
payload. Socket-create never allocates a host descriptor; failed connects
leave the reserved ID available for retry without sleeping.

A rejected oversized header drains its declared body without retaining it;
its payload cannot be mistaken for later commands. Zero declared length is
an error. Interframe padding is skipped only at a header boundary when enabled.
`finish` reports truncated input and resets framing. There is no attempt to
resynchronize inside a declared body. Callers own output/event retention.

PPP input events replace every parameter byte with zero, including malformed
requests. Config's PPP Debug and snapshots expose only whether credentials
are configured. No event exposes a password. These interfaces do not promise
memory zeroization. PDC reception comes from config on power-on; the initial
off state has PDC 03 and reception 15. Power-on resets PPP/DNS/phone/socket
state, while a second power-on when already on preserves it.

`backend::Tcp` uses IPv4 system sockets. Its default resolver accepts numeric
dotted IPv4 addresses and explicitly mapped names. Unlisted names are denied;
System DNS requires `TcpOptions::allow_system_dns = true` or an exact
hostname redirect that explicitly authorizes resolution of its target.
Resolution order is exact map, numeric address, `resolve_all`, then authorized
system DNS. Zero IPv4 is denied. Names match exact bytes; the system resolver
may apply its own normalization only after opt-in. That resolver's OS wait
policy is separate from the connect timeout.

Connect, receive and send timeouts default to five seconds. Each receive owns
one socket wait; readline uses successive one-byte receives through LF.
Timeout/errors yield `Recv::Error` and the core's `00 FF` read failure.
`max_receive` caps even direct oversized trait calls at 64 KiB. Zero-size
receives consume nothing. Writes may be partial. Close/drop shuts down and
releases the stream. A failed connect returns immediately without the C
adapter's extra five-second sleep.

Hostname redirects use the first exact match, with no chaining, wildcard,
case conversion or suffix matching. DNS replies echo the redirected name;
the `Network` trait carries IPv4 only, so arbitrary OS canonical-name rewriting
is not exposed. `Network::resolve_redirect` grants live DNS permission only
for the selected target of an explicit rule, without enabling later unlisted
lookups; its default implementation uses ordinary resolution for injected
backends. Socket redirects use the first matching IP and source port;
source port zero matches any port. A nonzero requested port is preserved by
an IP-only rule, even if the target has a port. On exact port equality the
target port replaces it, including target zero. These rules come from the
pinned `wongate_hack.c`; bounds checks replace its unchecked text parsing.

Built-in BPL recognizes the infrastructure names listed in the crate's
`data/bpl-hosts.txt` and resolves them locally before hostname redirects.
Connections to `127.0.0.1:5555` are intercepted before socket redirects or
backend I/O, matching C. Disabling `builtin_bpl` delegates both operations to
the configured redirects/backend. There is no BPL network listener or URL
fetcher. Request bodies are opaque and discarded as their declared lengths
arrive. Only the 24-byte header and a bounded FIFO of replies are retained.

Each complete BPL message produces one 24-byte field-preserving allow reply.
Fragmented headers/bodies and consecutive messages work. An incomplete request
produces no data; a positive read returns the profile's zero/FF convention
without latching transport EOF, so later writes can complete another message.
Zero-size reads preserve queued replies. C silently drops a consecutive
reply if the previous one is unread; this implementation queues all replies
within `max_bpl_pending`. Version/header-size mismatch, total below 24, total
above `max_bpl_message`, or queue overflow fails that connection. These strict
malformed-input and queue rules intentionally replace C's total coercion,
unbounded body count and dropped responses. Both BPL limits default to 64 KiB;
`BplAllow::finish` diagnoses an incomplete message.

`Config::from_ini` accepts `[pdc] reception`, `[ppp] user/pass`,
`[bpl_proxy] enabled`, `[hostname_hack] source = target`, and
`[socket_hack] IPv4[:port] = IPv4[:port]`. Empty/missing PPP is safe.
Names/sections are case-sensitive. Whitespace, CRLF, a leading UTF-8 BOM,
full-line `;`/`#` comments and whitespace-prefixed inline comments work.
Values are unquoted; embedded punctuation is preserved. Ordinary duplicate
keys use the last value; redirect entries retain source order. Continuation
lines and colon-only key separators are unsupported. Unknown sections/keys,
NUL, incomplete tokens and out-of-range values are errors, with line numbers
and no echoed input values. Text is bounded to 1 MiB, lines to 8192 bytes and
credentials to 255 bytes. Reception is checked in 0..15.

Standalone detector handling is opt-in. `probe=echo` returns one raw `8F` for
each standalone `8F`; `probe=ignore` discards it, allowing clients that enter
the framed session directly to avoid an unread detector reply. Both policies
apply only between frames, including after a rejected frame's declared body
has drained. Header, command and payload bytes are never intercepted. Neither
policy powers on the adapter or contacts a backend. The default `framed`
retains the original decoder behavior. These are client compatibility policies,
not a measurement of a physical adapter. Initialization still returns ASCII
major/minor `10` in both profiles.

`zero_sync=true` enables a client warm-up sequence: ten consecutive raw zero
bytes between frames receive five raw zero bytes. Native client instructions
send ten zeros and require five successful reads without testing their values.
The reply is a revival placeholder; physical adapter reply values are unknown.
The counter survives input chunk boundaries and resets on any other byte.
Frame headers, payloads and rejected-frame drains remain opaque. This policy
requires the profile's zero-padding behavior and performs no power or network
action. It is disabled by default.

`dial_padding=true` appends a zero parameter to a successful `01:08` reply.
An older client reader decrements the remaining length twice around its status
read; the extra declared byte prevents its drain counter from underflowing.
The zero is a compatibility placeholder and can remain unread until the next
header scan. Default dial replies retain one result parameter.

`dial_min_length=N` pads a successful `01:08` reply with trailing zeros until
the total reply frame reaches N bytes. A client reader that validates a
minimum acknowledgment length rejects shorter replies; the padding bytes are
compatibility placeholders and can remain unread. Zero disables the floor and
preserves default replies.

Numeric values accept decimal, `0x`, `$`, `0b` and `0o` prefixes. Complete
tokens and checked arithmetic replace C's suffix truncation and overflow.
IPv4 uses dotted four-octet notation. Boolean fields accept `true`, `false`
or numeric zero/nonzero. `[swankit]` provides the following settings:

| Key | Default / valid values |
| --- | --- |
| `profile` | `revival`; also `upstream` |
| `probe` | `framed`; opt-in `ignore` or `echo` for standalone `8F` at a frame boundary |
| `zero_sync` | false; opt-in five-byte raw reply to ten boundary-padding zeros |
| `dial_padding` | false; opt-in trailing zero on successful `01:08` acknowledgments |
| `dial_min_length` | 0; 0..255 total reply bytes on successful `01:08` acknowledgments |
| `header` | profile default; `len16be` or `zero-then-len8`, independent of key order |
| `backend` | `tcp`; also `transcript` (selection metadata for the caller) |
| `transcript_file` | optional external path, loaded by the caller |
| `max_frame` | 4096; 1..65535, then capped by header mode |
| `max_sockets` | 255; 0..255 |
| `max_bpl_message`, `max_bpl_pending` | 65536 each; 24..65536 |
| `max_receive` | 65536; 1..65536 |
| `connect_timeout_ms`, `recv_timeout_ms`, `send_timeout_ms` | 5000 each; 1..600000 |
| `allow_system_dns` | false; explicit opt-in only |
| `resolve_all` | absent; explicit dotted IPv4 catch-all |
| `resolve.NAME` | absent; explicit dotted IPv4 for that exact name |

```ini
[hostname_hack]
service.invalid = 127.0.0.1
[socket_hack]
192.0.2.1:80 = 127.0.0.1:8080
[swankit]
profile = revival
backend = tcp
allow_system_dns = false
recv_timeout_ms = 5000
```

`transcript::Tap` writes `T <ms> > <hex>` for console-to-adapter bytes and
`T <ms> < <hex>` for replies, flushing each record. `write_at` takes an explicit
timestamp for deterministic output; `record` uses elapsed milliseconds.
`parse`/`read` join each direction and retain timestamps/ranges without copying
the payload into each record. Wrapper start/end `T <ms> # ...` metadata is
ignored. Malformed hex/records are errors; configurable limits cap input text,
joined streams and record counts. `frames` uses the core Decoder, both header
modes and optional padding, returning wire offsets or a checked framing error.
Tap records literal bytes; opt-in raw logs may contain PPP input, unlike
redacted `Event`/Debug output. Logs belong outside the repository.

`backend::Transcript` queues `TranscriptPeer` request/response pairs with
exact endpoints, from bounded external files, vectors or TCP-level tap streams.
It never opens host sockets. Fragmented sends must match the expected request
prefix exactly; wrong/extra bytes or reading before the complete request fails
the connection. Responses can be read in arbitrary fragments and then give
EOF. Each connection has separate positions. `TranscriptLog::verify`, called
after close/drop, rejects mismatches, short requests and unused peers; unread
response tails are allowed for clients that deliberately stop early.
The replay budget covers at most 16 MiB of request/response bytes and 4096
connections per run. TCP-level pair streams differ from console serial taps;
adapter golden enrollment and its wire manifest belong to M15.

`tools/wg-differential.sh upstream|revival` takes `WG_C_ADAPTER`, an independent
compiled C binary. Upstream uses the pinned unpatched source; revival needs
the original small-ID harness patch and patches 01–07. It does not build or
modify C. A small Rust example exposes the core over stdin/stdout solely for
this optional external gate; the production CLI is M14. `WG_RUST_ADAPTER` can
select that prebuilt example. `WG_STREAMS` defaults to 10000 and `WG_SEED` to
`0x13c0ffee`. Each stream compares stdout bytes and peer request bytes, with
random valid control commands, DNS/redirects, TCP fragments, line/block reads,
EOF, zero-size reads, close and BPL messages. Unknown commands are flushed by
a known marker rather than a timing-based absence check. Peer readiness
precedes reads to avoid TCP delivery segmentation deciding the comparison.

The differential excludes malformed C inputs, negative upstream receives and
length/count overflow because they trigger undefined behavior rather than an
oracle result. PPP config is explicit to avoid C's `strdup(NULL)` defect.
Zero-size TCP comparison reads occur after data/FIN readiness: C's zero-length
`recv` can block before then, while Rust always consumes nothing. Unpatched C's
stdin EOF spin is handled by stopping only the owned child after all bytes
are compared. Run both profiles externally for milestone sign-off; CI keeps
this gate optional. Synthetic malformed/limit tests remain independent of C.
The library adds no public service, publishing or hardware-fidelity claim.

## Shared server core

The server's BPL responder accepts opaque incoming version/header-size metadata, checks the declared total
length, and emits a standardized version-1, 24-byte reply header. It bounds the
LE32 total to 24..65536, waits for the entire declared message, copies bytes
8..23, and writes the allow header at 0..7. Commands and URL records are opaque.
Malformed lengths and partial EOF return errors; TCP closes the connection.
Several valid messages can share one TCP connection, each with its own log.
The legacy eight-byte Python response is deliberately corrected; there is no
eight-byte success profile for the server. Adapter frame modes remain separate.

The shared HTTP port preserves binary bodies, validates unique ASCII header
names and exact Content-Length (no transfer encoding), accepts absolute or
origin requests and a data-declared platform alias pair, and tolerates exactly
one extra NUL only with zero declared body length. Limits cap headers at 16 KiB
and bodies at 64 KiB before allocation. Responses have fixed header order.
Charge policy is newly authored: zero cost, an open service window, stable
locally issued session tokens and free billing that preserves an incoming
printable 80-byte token. Service identities live in JSON, with no live DNS or
outbound connections in the server. See [server API and storage policy](../crates/ws-wondergate-server/README.md).

## Data-defined title services

The definition engine composes bundled JSON with shared services. Bounded
segments, phase values, percent/form fields, scoped ranking actions and checksum
envelopes preserve each client lineage's wire contract. Histories and static
selections carry new provenance. The [definition read/write guide](wondergate-title-definitions.md)
explains load-time validation, client response limits and the empty plugin ledger.
Golden native HTTP bytes remain external; ordinary tests use synthetic requests.

TCP receive timeouts return the current read error reply while allowing the
next read to retry the same socket. EOF and fatal receive errors retain their
terminal behavior. Appended raw taps carry start/end markers; `wg-decode
--session N` selects a one-based session so earlier incomplete frames do not
merge into its input.
