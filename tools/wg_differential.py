#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Seeded, valid command streams over owned loopback peers and two adapters.

C is deliberately left unmodified by this tool. Upstream's negative-read UB,
overflowing replies, unterminated names and missing PPP config are excluded;
zero-byte TCP reads are issued only once data/FIN is ready, since C can block
on a zero-length recv while its TCP peer is still waiting for the request.
Rust-only malformed/limit tests cover them. C's stdin EOF spin is handled by
terminating only the child this driver started, after comparing all output.
"""
import argparse
import hashlib
import os
from pathlib import Path
import queue
import random
import select
import socket
import struct
import subprocess
import tempfile
import threading
import time


def frame(kind, command, params=b""):
    length = len(params) + 1
    if not 1 <= length <= 255:
        raise ValueError("differential fixtures must fit legacy framing")
    return bytes((kind, 0, length, command)) + params


class Child:
    def __init__(self, binary, config):
        self.process = subprocess.Popen([str(Path(binary).resolve()), str(config)],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL)
        self.digest = hashlib.sha256()
        self.bytes = 0

    def receive(self, count):
        data = bytearray()
        end = time.monotonic() + 5
        while len(data) < count:
            remaining = end - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise AssertionError("adapter reply timeout")
            part = os.read(self.process.stdout.fileno(), count - len(data))
            if not part:
                raise AssertionError("adapter exited before its reply")
            data.extend(part)
        return bytes(data)

    def reply(self):
        head = self.receive(3)
        result = head + self.receive(int.from_bytes(head[1:3], "big"))
        self.digest.update(result)
        self.bytes += len(result)
        return result

    def send(self, data):
        self.process.stdin.write(data)
        self.process.stdin.flush()

    def close(self):
        # Upstream C has a known EOF spin. Never target a process not owned here.
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.communicate(timeout=3)


class Peers:
    def __init__(self):
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.listener.settimeout(5)
        self.port = self.listener.getsockname()[1]
        self.jobs = queue.Queue()
        self.errors = queue.Queue()
        self.worker = threading.Thread(target=self.serve, daemon=True)
        self.worker.start()
        self.connections = 0

    def pair(self, request, response):
        ready = [threading.Event(), threading.Event()]
        for event in ready:
            self.jobs.put((request, response, event))
        return ready

    def serve(self):
        while True:
            job = self.jobs.get()
            if job is None:
                return
            expected, response, ready = job
            try:
                with self.listener.accept()[0] as peer:
                    peer.settimeout(5)
                    received = bytearray()
                    while len(received) < len(expected):
                        part = peer.recv(len(expected) - len(received))
                        if not part:
                            raise AssertionError("short loopback request")
                        received.extend(part)
                    if bytes(received) != expected:
                        raise AssertionError("loopback request bytes differ")
                    peer.sendall(response)
                    peer.shutdown(socket.SHUT_WR)
                    self.connections += 1
            except Exception as error:
                self.errors.put(error)
            finally:
                ready.set()

    def wait(self, events):
        for event in events:
            if not event.wait(5):
                raise AssertionError("loopback response timeout")
        if not self.errors.empty():
            raise self.errors.get()

    def close(self):
        self.jobs.put(None)
        self.listener.close()
        self.worker.join(timeout=6)


class Differential:
    def __init__(self, c, rust, profile, peers, rng):
        self.c, self.rust, self.profile, self.peers, self.rng = c, rust, profile, peers, rng
        self.commands = 0
        self.bpl_messages = 0
        self.stream = 0
        self.input_digest = hashlib.sha256()

    def command(self, kind, cmd, params=b"", reply=True):
        wire = frame(kind, cmd, params)
        # Unknown commands are flushed with a known marker; their lack of a
        # reply is tested without sleeping or depending on scheduling.
        if not reply:
            wire += frame(2, 1, b"\x03")
        self.input_digest.update(wire)
        self.c.send(wire)
        self.rust.send(wire)
        try:
            left, right = self.c.reply(), self.rust.reply()
        except AssertionError as error:
            raise AssertionError(f"stream={self.stream} input={wire.hex()}: {error}") from error
        self.commands += 1 + (not reply)
        if left != right:
            raise AssertionError(f"stream={self.stream} command={kind:02x}:{cmd:02x} "
                                 f"input={wire.hex()} C={left.hex()} Rust={right.hex()}")
        return right

    def send_payload(self, socket_id, payload):
        while payload:
            size = self.rng.randint(1, min(len(payload), 48))
            self.command(0x11, 0x0e, bytes((socket_id,)) + payload[:size])
            payload = payload[size:]

    def random_control(self):
        rng = self.rng
        choice = rng.randrange(12)
        if choice == 0:
            self.command(1, 2)
        elif choice == 1:
            self.command(1, 0)
        elif choice == 2:
            self.command(2, 1, bytes((rng.choice([0, 3]) if self.profile == "revival" else 3,)))
        elif choice == 3:
            user, password = rng.randbytes(rng.randrange(8)), rng.randbytes(rng.randrange(8))
            self.command(1, 0x10, bytes((7, 0, 0, 0, 0, 1, len(user))) + user + bytes((len(password),)) + password)
        elif choice == 4:
            self.command(1, 0x11, b"\x04" + bytes(8) + rng.randbytes(8))
        elif choice == 5:
            digits = b"#" * rng.randrange(1, 8)
            mode = rng.choice([1, 3]) if self.profile == "revival" else 3
            self.command(1, 8, bytes((0, 1, mode, len(digits))) + digits)
        elif choice == 6:
            self.command(1, 0x0a, b"\x01")
        elif choice == 7:
            self.command(1, rng.choice([1, 3, 4, 9]), reply=self.profile == "revival")
        elif choice == 8:
            self.command(0x90, rng.randrange(256), rng.randbytes(rng.randrange(12)), reply=False)
        elif choice == 9:
            self.command(0x11, 8, rng.choice([b"127.0.0.1\0", b"redirect.invalid\0"]))
        elif choice == 10:
            self.command(0x11, 6, b"\xff\x02")
        else:
            self.command(0x11, 7, b"\xff")

    def connect(self, socket_id, ip, port):
        self.command(0x11, 3, bytes((socket_id, 0, self.rng.choice([0, 2]))) + struct.pack("<H", port) + ip + bytes(8))

    def run_stream(self, index, bpl_hosts):
        self.stream = index
        rng = self.rng
        self.command(15, 255, b"\x55\xaa\x55\xaa")
        self.command(1, 2)
        for _ in range(rng.randint(3, 12)):
            self.random_control()
        socket_id = self.command(0x11, 1, b"\0")[-1]
        first_id = 0 if self.profile == "revival" else 48
        if socket_id != first_id:
            raise AssertionError("unexpected first socket id")
        request = b"request:" + rng.randbytes(rng.randint(1, 120))
        response = b"alpha\r\n" + b"x" * rng.randint(1, 140) + b"\r\n\r\n" + rng.randbytes(rng.randint(1, 96))
        events = self.peers.pair(request, response)
        if index % 2:
            self.connect(socket_id, bytes((192, 0, 2, 1)), 80)
        else:
            self.connect(socket_id, bytes((127, 0, 0, 1)), self.peers.port)
        self.command(0x11, 6, bytes((socket_id, rng.randrange(3))))
        self.send_payload(socket_id, request)
        self.peers.wait(events)  # All response bytes and FIN queued before reads.
        for cmd in [0x0d, 0x0f]:
            self.command(0x11, cmd, bytes((socket_id, 0, 0)))
        consumed = 0
        while consumed < len(response):
            cmd = rng.choice([0x0d, 0x0f])
            reply = self.command(0x11, cmd, bytes((socket_id, rng.randint(1, 96), 0)))
            count = reply[5]
            if not 0 < count < 255 or reply[6:] != response[consumed:consumed + count]:
                raise AssertionError("unexpected loopback read bytes")
            consumed += count
        for cmd in [0x0d, 0x0f]:
            self.command(0x11, cmd, bytes((socket_id, 20, 0)))
            self.command(0x11, cmd, bytes((socket_id, 0, 0)))
        self.command(0x11, 7, bytes((socket_id,)))
        self.command(0x11, 7, bytes((socket_id,)))
        if self.profile == "revival":
            self.command(0x11, 0x0f, b"\xff\x01\0")
        if index % 4 == 0:
            self.command(0x11, 8, rng.choice(bpl_hosts) + b"\0")
            self.command(0x11, 1, b"\0")
            self.connect(socket_id, bytes((127, 0, 0, 1)), 5555)
            for _ in range(rng.randint(1, 3)):
                body = rng.randbytes(rng.randrange(48))
                request = b"\x01\x18" + rng.randbytes(2) + struct.pack("<I", 24 + len(body)) + rng.randbytes(16) + body
                self.send_payload(socket_id, request)
                wanted = bytes.fromhex("0118100018000000") + request[8:24]
                received = bytearray()
                while len(received) < 24:
                    reply = self.command(0x11, 0x0f, bytes((socket_id, rng.randint(1, 24))))
                    received.extend(reply[6:])
                if bytes(received) != wanted:
                    raise AssertionError("BPL reply differs from the field-preserving contract")
                self.command(0x11, 0x0f, bytes((socket_id, 24)))
                self.bpl_messages += 1
            self.command(0x11, 7, bytes((socket_id,)))
        self.command(15, 255, b"\x55\xaa\x55\xaa")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--c", required=True)
    parser.add_argument("--rust", required=True)
    parser.add_argument("--profile", choices=("upstream", "revival"), required=True)
    parser.add_argument("--streams", type=int, default=10000)
    parser.add_argument("--seed", type=lambda text: int(text, 0), default=0x13c0ffee)
    args = parser.parse_args()
    if not 1 <= args.streams <= 1000000:
        parser.error("streams must be 1..1000000")
    hosts = (Path(__file__).resolve().parents[1] / "crates/ws-wondergate/data/bpl-hosts.txt").read_bytes().splitlines()
    c = rust = peers = None
    try:
        peers = Peers()
        with tempfile.TemporaryDirectory(prefix="wg-differential-") as directory:
            config = Path(directory) / "wonfence.ini"
            config.write_text("[ppp]\nuser=unit\npass=unit\n[hostname_hack]\nredirect.invalid=127.0.0.1\n"
                              f"[socket_hack]\n192.0.2.1:80=127.0.0.1:{peers.port}\n"
                              f"[swankit]\nprofile={args.profile}\n")
            c, rust = Child(args.c, config), Child(args.rust, config)
            runner = Differential(c, rust, args.profile, peers, random.Random(args.seed))
            for index in range(args.streams):
                runner.run_stream(index, hosts)
                if (index + 1) % 1000 == 0:
                    print(f"{args.profile}: {index + 1}/{args.streams} streams matched", flush=True)
            if select.select([c.process.stdout, rust.process.stdout], [], [], 0)[0]:
                raise AssertionError("unexpected trailing stdout")
            print(f"PASS profile={args.profile} seed={args.seed:#x} streams={args.streams} "
                  f"commands={runner.commands} tcp_connections={peers.connections} "
                  f"bpl_messages={runner.bpl_messages} stdout_bytes={c.bytes} "
                  f"C_sha256={c.digest.hexdigest()} Rust_sha256={rust.digest.hexdigest()} "
                  f"input_sha256={runner.input_digest.hexdigest()}", flush=True)
    finally:
        for child in (c, rust):
            if child is not None:
                child.close()
        if peers is not None:
            peers.close()


if __name__ == "__main__":
    main()
