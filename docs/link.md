# Serial link relay

Build with `cargo build -p swankit-cli --locked`, then use `swankit link A B`.
The relay carries raw data bytes between two serial cable ends. It does not
carry baud changes, clock timing, UART status or framing errors. Each emulator
or device controls its own serial pacing and receive behavior.

| Endpoint | Behavior |
| --- | --- |
| `tcp:HOST:PORT` | Connect and retry every second, also after disconnect; TCP_NODELAY |
| `listen:[HOST:]PORT` | Accept one peer at a time; omitted host means `127.0.0.1` |
| `stdio` | Binary stdin/stdout; diagnostics stay on stderr; allowed once |
| `exec:COMMAND` | `/bin/sh -c COMMAND` with binary pipes; owned process group terminated on exit |
| `device:PATH` | Linux raw serial device, 8N1 with no flow control |
| `wondergate[:CONFIG.ini]` | Existing adapter core with the configured TCP backend |

The relay starts reading only when both ends are connected. At startup and
after disconnect, early data remains in OS socket/pipe buffers. Its pending
queues hold at most 65,536 bytes per direction and apply backpressure when full.
A disconnect drops only bytes already read into those queues, recording each
drop; the surviving end's unread bytes wait for reconnect. Fixed endpoints
(stdio, exec, device and adapter) cannot reopen. When two fixed ends cannot
resume, the relay exits with an error. A TCP/listen pair keeps waiting for
returning peers; use Ctrl-C to end it.

## Two emulators over TCP

```sh
swankit link listen:3000 listen:3001 --log cable.tsv
```

Configure one emulator's TCP serial endpoint to connect to `127.0.0.1:3000`
and the other's to `127.0.0.1:3001`. For another machine, bind an explicit host
address and use that address at the client. The default listeners are loopback
only. The stream has no authentication or encryption.

An emulator with a TCP serial endpoint passed a complete binary echo test in
both directions and a full save transfer through this relay, including the
delay settings below. These are byte-stream interoperability results rather
than cycle-accurate serial emulation claims.

## An emulator that forks a serial helper

Create an executable wrapper on PATH:

```sh
#!/bin/sh
exec swankit link stdio tcp:127.0.0.1:3000 --log helper.tsv
```

The wrapper ignores any extra emulator arguments. Its stdout must contain only
serial bytes. Start the cable relay, configure the other emulator to connect
to port 3001, then launch the helper-based emulator:

```sh
swankit link listen:3000 listen:3001 --log cable.tsv
mednafen -wswan.excomm 1 -wswan.excomm.path "$SERIAL_HELPER" "$TEST_ROM"
```

Stock Debian Mednafen `1.32.1+dfsg-2+b1` passed the echo fixture with control
writes A0/E0: 515 bytes in each direction, zero relay drops, and the TCP peer's
completion marker reported 512 echoed bytes and one receive interrupt. This
supports received-byte latching with bit 5 set. Behavior with bit 5 clear was
not tested; the known bit-5 receive-latch limitation is not a failure observed
in this run. UART behavior remains the emulator's responsibility. No emulator
patch is needed for this fixture, and no emulator code is part of the relay.

For a headless container, create the `.X11-unix` socket directory in the
container's temporary directory with mode 1777 before running as a non-root
user. Start Xvfb on a high unused display such as
`:97` with `-nolisten tcp`. In the tested container, `xvfb-run` waited without
launching the emulator; starting Xvfb directly, waiting for its Unix socket,
and setting `DISPLAY=:97` worked. Host networking kept the loopback relay
reachable. Emulator result bytes, rather than controlled container shutdown
statuses, were the completion oracle.

For a simple echo peer without a second emulator:

```sh
swankit link listen:3000 'exec:cat' --log echo.tsv
```

## USB serial adapter and a console

```sh
swankit link device:/dev/ttyUSB0 listen:3000 --baud 9600 --log device.tsv
```

Connect the emulator's TCP serial stream to port 3000. `--baud` applies to device
endpoints only and accepts 9600, 38400 and 192000. The two console UART rates,
9600 and 38400, and EXT port semantics are documented by
[WSdev UART](https://ws.nesdev.org/wiki/UART). 192000 is an adapter-side rate,
not a third console UART mode. Linux termios2 configures these device rates.
An appropriate console EXT adapter must present a serial byte stream.
PTY tests verified raw control/binary bytes both ways at all accepted rates;
physical adapters, console wiring and real hardware are untested.

## Delay and jitter

```sh
swankit link listen:3000 listen:3001 --latency-ms 20 --log delay.tsv
swankit link listen:3000 listen:3001 --latency-ms 100 \
  --jitter-ms 30 --seed 22 --log jitter.tsv
```

Latency and jitter are bounded to 600000 ms. Jitter is uniform within the
requested positive/negative interval; negative total delays clamp to zero.
Each direction has its own deterministic seeded sequence. Deadlines never
move backwards, preserving byte order. Buffering and peer pacing can increase
the observed delay. A recorded full-memory transfer passed at 0 ms, 20 ms,
and 100 ms with 30 ms jitter. This does not establish tolerance for other games.

## WonderGate adapter

```sh
swankit link listen:3000 wondergate:adapter.ini --log adapter.tsv
```

The existing adapter owns command parsing and replies; its configured TCP
backend owns network requests. See [WonderGate configuration](wondergate.md).
The relay adds no adapter features or hardware timing. A console emulator
reached a local HTTP server through this endpoint during interoperability
testing; displayed-page fidelity was not separately assessed.
An adapter worker may finish an in-progress network wait after cancellation,
within the configured backend timeouts.

## Logs

`--log` requires a new file; existing logs are never overwritten. Without it,
byte records are discarded. The TSV header is `direction time_us value result`
with tabs between columns. Directions are `A>B` and `B>A`, values use two hex
digits, and results are `FORWARDED` or `DROPPED: reason`. Connection events start
with `#`. Timestamps measure microseconds since relay start at forwarding/drop
time; they cannot establish per-byte arrival-to-delivery latency by themselves.
