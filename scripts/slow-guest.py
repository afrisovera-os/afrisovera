#!/usr/bin/env python3
"""A guest that is SLOW but never WRONG.

This is the 2.757 failure mode in a bottle. The three reds that run recorded had
captures that were truncations of CORRECT output — four of five commands answered
right, then silence — which is the signature of a driver giving up on a starved
guest, not of a subsystem regressing. Reproducing that with real host contention
takes a load heavy enough to matter to whoever owns the machine, and it is not
repeatable. So instead: a serial peer that speaks the shell's protocol exactly,
answers every command with the RIGHT answer, and simply takes DELAY seconds to do
it. Any driver that records this as a failure is recording a fact about its own
patience, not about the guest.

Also serves the minimal QMP handshake shell-demo.py performs, so the driver under
test runs unmodified.
"""
import json
import socket
import sys
import threading
import time

DELAY = float(sys.argv[1])
SER_PORT = int(sys.argv[2])
QMP_PORT = int(sys.argv[3])

ANSWERS = {
    "write hf.txt abcdefgh": "wrote 8 bytes",
    "head 3 hf.txt": "head: abc",
    "head 100 hf.txt": "head: abcdefgh",
    "head 3 nope.txt": "head: nope.txt not found",
}


def serial():
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", SER_PORT))
    s.listen(1)
    c, _ = s.accept()
    c.sendall(b"AfriSovera / mansa-kernel boot ok\r\nafrisovera> ")
    buf = b""
    while True:
        try:
            b = c.recv(4096)
        except OSError:
            break
        if not b:
            break
        buf += b
        while b"\r" in buf:
            line, buf = buf.split(b"\r", 1)
            cmd = line.decode(errors="replace").strip()
            c.sendall(cmd.encode() + b"\r\n")      # echo, as the shell does
            time.sleep(DELAY)                      # the whole point
            ans = ANSWERS.get(cmd, f"unknown command: {cmd}")
            c.sendall(ans.encode() + b"\r\nafrisovera> ")


def qmp():
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", QMP_PORT))
    s.listen(1)
    c, _ = s.accept()
    f = c.makefile("rwb")
    f.write(json.dumps({"QMP": {"version": {}, "capabilities": []}}).encode() + b"\n")
    f.flush()
    while True:
        line = f.readline()
        if not line:
            break
        f.write(b'{"return": {}}\n')
        f.flush()


threading.Thread(target=qmp, daemon=True).start()
serial()
