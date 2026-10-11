"""Observable regressions against the real CLI; all processes are owned here."""
import fcntl
import os
import pty
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest

BINARY = sys.argv.pop(1)

class Link(unittest.TestCase):
    def port(self):
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            return s.getsockname()[1]

    def launch(self, *args, **kw):
        p = subprocess.Popen([BINARY, 'link', *args], stderr=subprocess.PIPE, **kw)
        self.addCleanup(self.cleanup, p)
        return p

    def cleanup(self, p):
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(timeout=2)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait(timeout=2)
        for pipe in (p.stdin, p.stdout, p.stderr):
            if pipe:
                pipe.close()

    def connect(self, port):
        deadline = time.monotonic() + 2
        while True:
            try:
                s = socket.create_connection(('127.0.0.1', port), timeout=1)
                self.addCleanup(s.close)
                s.settimeout(2)
                return s
            except ConnectionRefusedError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.01)

    def test_stdio_listen_exit(self):
        p = self.launch('stdio', f'listen:{self.port()}', stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE)
        p.stdin.close()
        try:
            status = p.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.fail('permanent stdio EOF left relay alive with reconnectable listen partner')
        self.assertNotEqual(status, 0)

    def test_exec_tcp_exit_and_children(self):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            listener.settimeout(2)
            p = self.launch('exec:printf early; sleep .2',
                            f'tcp:127.0.0.1:{listener.getsockname()[1]}')
            conn, _ = listener.accept()
            with conn:
                conn.settimeout(2)
                self.assertEqual(conn.recv(5), b'early')
                self.assertEqual(conn.recv(1), b'')
            self.assertNotEqual(p.wait(timeout=2), 0)

    def test_sighup_reaps_child_group(self):
        with tempfile.TemporaryDirectory() as d:
            pidfile = os.path.join(d, 'child.pid')
            p = self.launch(f'exec:echo $$ > {pidfile}; exec sleep 60',
                            f'listen:{self.port()}')
            deadline = time.monotonic() + 2
            while not os.path.exists(pidfile) and time.monotonic() < deadline:
                time.sleep(.01)
            with open(pidfile) as f:
                child = int(f.read())
            def cleanup_child():
                try:
                    os.kill(child, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            self.addCleanup(cleanup_child)
            p.send_signal(signal.SIGHUP)
            p.wait(timeout=2)
            try:
                os.kill(child, 0)
            except ProcessLookupError:
                return
            self.fail('SIGHUP left the owned exec child alive')

    def test_pty_shared_flags_restored(self):
        master, slave = pty.openpty()
        self.addCleanup(os.close, master)
        self.addCleanup(os.close, slave)
        original = fcntl.fcntl(slave, fcntl.F_GETFL)
        p = self.launch('stdio', 'exec:sleep .2', stdin=slave, stdout=slave)
        p.wait(timeout=2)
        self.assertEqual(fcntl.fcntl(slave, fcntl.F_GETFL), original,
                         'relay left shared terminal descriptor flags changed')

    def test_early_bytes_then_close_before_partner(self):
        a, b = self.port(), self.port()
        p = self.launch(f'listen:{a}', f'listen:{b}')
        first = self.connect(a)
        first.sendall(b'buffered-before-partner')
        first.shutdown(socket.SHUT_WR)
        time.sleep(.2)
        second = self.connect(b)
        self.assertEqual(second.recv(100), b'buffered-before-partner')
        p.terminate()
        self.assertEqual(p.wait(timeout=2), 0)

    def test_wondergate_two_sessions(self):
        port = self.port()
        p = self.launch('wondergate', f'listen:{port}')
        first = self.connect(port)
        # Incomplete serial frame must not carry into the next emulator.
        first.sendall(bytes.fromhex('01 00 02 02'))
        first.close()
        time.sleep(.2)
        second = self.connect(port)
        # Command 01:01 returns a zero parameter in the fresh session.
        second.sendall(bytes.fromhex('01 00 01 01'))
        self.assertEqual(second.recv(20), bytes.fromhex('01 00 02 01 00'))
        p.terminate()
        self.assertEqual(p.wait(timeout=2), 0)

if __name__ == '__main__':
    unittest.main()
