# WonderGate adapter core

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
M12 transports these bytes opaquely; the responder belongs to M13.

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
| poll_reports_allocation | false | true |
| dial_modes | 03 | 01, 03 |
| status_params | 03 | 00, 03 |
| bpl_reply | FieldPreserving24 | FieldPreserving24 |

Each field is individually selectable. Profiles reproduce the pinned MIT
WonderFence command rules and the seven local client-derived corrections.
Small IDs, empty metadata, incoming-call success and allocation-only poll
semantics remain compatibility policy. Legacy poll ignores parameters, matching its C handler. Strict validation of malformed
parameters and checked lengths intentionally replace upstream unsafe reads.
In legacy mode byte 1 is ignored on input, matching C; output writes zero.
The internal C no-reply marker never becomes a length flag.

The following table lists **reply parameters**, following `[type][BE length]
`[command]`. Results/IDs/counts are byte-sized; IPv4 order is network order.

| Request | Parameters / action | Reply command and parameters |
| --- | --- | --- |
| 01:02 | power on; reset only on off-to-on transition | 02, ASCII `10` |
| 01:00 | check PDC | 00, `[pdc][reception]` |
| 02:01 | status selector from profile | **02**, `[status]` (off 00, on 02, PPP 03) |
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

INI, redirects, TCP, transcript replay and a built-in BPL responder are later
milestones. Adapter corpus goldens are enrolled separately; M12's evidence is
synthetic wire tests and client-code analysis.
