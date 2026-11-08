#!/usr/bin/env python3
"""Talk to the guest's shell over QEMU's emulated USB serial cable (FTDI, `usb-serial`).

    usb-serial-talk.py <COM1 port> <USB serial port> <COM1 out> <USB out> <command>...

Both are QEMU `server,nowait` tcp chardevs. The USB serial socket is connected at once (QEMU
plugs the cable in when its chardev opens); COM1 is only listened to. Once COM1 shows the shell
prompt and the kernel's "USB serial cable" line, each <command> is typed on the USB cable
(CR after it, as a terminal sends) and the cable is read until the next prompt comes back on
it. Nothing is typed on COM1: whatever the shell runs, it read from the USB cable, and what
the far end of the cable sees is the console mirrored to it. Both captures are written out.
Used by scripts/pc-usbserial-test.sh.
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
    raise SystemExit(f"usb-serial-talk: nothing listening on {port}")


def main():
    cport, uport, cout, uout = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4]
    cmds = sys.argv[5:]
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
        raise SystemExit("usb-serial-talk: no shell prompt and USB serial cable on COM1")
    pump(2)
    for c in cmds:
        mark = cap[usb].count(b"afrisovera> ")
        for ch in (c + "\r").encode():
            usb.sendall(bytes([ch]))
            time.sleep(0.02)
        if not pump(30, lambda: cap[usb].count(b"afrisovera> ") > mark):
            print(f"usb-serial-talk: no prompt came back on the cable after {c!r}")
        pump(1)
    save()


if __name__ == "__main__":
    main()
