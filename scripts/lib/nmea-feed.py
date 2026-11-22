#!/usr/bin/env python3
"""Play a GPS receiver on QEMU's emulated USB serial cable (FTDI, `usb-serial`) and type on COM1 (sensors S1).

    nmea-feed.py <COM1 port> <USB serial port> <COM1 out> <USB out> <step>...

Both are QEMU `server,nowait` tcp chardevs. The USB serial socket is connected at once (QEMU plugs the cable in
when its chardev opens). Once COM1 shows the shell prompt and the kernel's "USB serial cable" line, each step runs:
- `type:<command>`: typed on COM1 (CR after it), then COM1 is read until the next prompt;
- `send:<file>`: the file's NMEA lines sent on the cable, each ended with CR LF as a receiver ends them, 50 ms
  apart, then a second for the kernel's poll to read them.
Both captures are written out: COM1, and what the guest sent to the cable (the console mirrored to it, until the
cable is known to be a GPS). Used by scripts/pc-gps-test.sh.
"""
import select
import socket
import sys
import time


def connect(port, deadline):
    while time.time() < deadline:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=2)
        except OSError:
            time.sleep(0.2)
    raise SystemExit(f"nmea-feed: nothing listening on {port}")


def main():
    cport, uport, cout, uout = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4]
    steps = sys.argv[5:]
    deadline = time.time() + 240
    usb = connect(uport, deadline)
    com = connect(cport, deadline)
    cap = {com: bytearray(), usb: bytearray()}

    def pump(seconds, done=lambda: False):
        end = time.time() + seconds
        while time.time() < end:
            if done():
                return True
            r, _, _ = select.select([com, usb], [], [], 0.2)
            for s in r:
                try:
                    b = s.recv(4096)
                except OSError:
                    b = b""
                if b:
                    cap[s].extend(b)
        return done()

    def save():
        open(cout, "wb").write(bytes(cap[com]))
        open(uout, "wb").write(bytes(cap[usb]))

    ok = pump(220, lambda: b"afrisovera> " in cap[com] and b"USB serial cable" in cap[com])
    if not ok:
        save()
        raise SystemExit("nmea-feed: no shell prompt and USB serial cable on COM1")
    pump(2)
    for step in steps:
        kind, _, arg = step.partition(":")
        if kind == "type":
            mark = cap[com].count(b"afrisovera> ")
            for ch in (arg + "\r").encode():
                com.sendall(bytes([ch]))
                time.sleep(0.02)
            if not pump(30, lambda: cap[com].count(b"afrisovera> ") > mark):
                print(f"nmea-feed: no prompt came back on COM1 after {arg!r}")
            pump(1)
        elif kind == "send":
            for line in open(arg, "rb").read().splitlines():
                line = line.strip()
                if line:
                    usb.sendall(line + b"\r\n")
                    pump(0.05)
            pump(1)
        else:
            raise SystemExit(f"nmea-feed: unknown step {step!r}")
    save()


if __name__ == "__main__":
    main()
