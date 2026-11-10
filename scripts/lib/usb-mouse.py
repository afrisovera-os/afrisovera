#!/usr/bin/env python3
"""Move and click QEMU's emulated USB mouse, then type commands on its USB keyboard.

    usb-mouse.py <serial port> <qmp port> <out file> <png file> <dx> <dy> <clicks> <text>...

Connects to the guest's serial console (tcp, QEMU `server,nowait`) and its QMP socket, waits
for the shell prompt, then through QMP `input-send-event` moves the mouse by <dx>,<dy> in
steps of at most 40 counts (QEMU gives them to the mouse the guest polls -- the USB one; its
`device` argument needs a display console, which -display none has not) and presses + releases the left button <clicks> times. Then every <text> is
typed on the USB keyboard (`send-key`, Enter after each), a screenshot is saved to
<png file>, and the whole serial capture is written to <out file>. Nothing is typed on the
serial line. Used by scripts/pc-usbmouse-test.sh.

With MOUSE_WHEEL set (e.g. "up up up down"), each word rolls the wheel one notch that way
after the clicks and before the typing (pc-ps2mouse-test.sh: the wheel scrolls the console).
"""
import json
import os
import socket
import sys
import time

QCODE = {" ": "spc", "-": "minus", "=": "equal", ".": "dot", ",": "comma", "/": "slash"}


def connect(port, deadline):
    while time.time() < deadline:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=2)
        except OSError:
            time.sleep(0.3)
    raise SystemExit(f"usb-mouse: nothing listening on {port}")


def main():
    sport, qport, out, png = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4]
    dx, dy, clicks = int(sys.argv[5]), int(sys.argv[6]), int(sys.argv[7])
    texts = sys.argv[8:]
    deadline = time.time() + 240
    ser = connect(sport, deadline)
    ser.settimeout(0.5)
    cap = bytearray()

    def drain(seconds, until=None):
        end = time.time() + seconds
        while time.time() < end:
            try:
                b = ser.recv(4096)
                if b:
                    cap.extend(b)
                    if until and until in cap:
                        return True
            except socket.timeout:
                pass
        return False

    if not drain(220, b"afrisovera> "):
        open(out, "wb").write(bytes(cap))
        raise SystemExit("usb-mouse: the shell prompt never appeared")
    drain(2)
    q = connect(qport, deadline)
    qf = q.makefile("rwb")
    qf.readline()  # greeting
    qf.write(b'{"execute":"qmp_capabilities"}\n')
    qf.flush()
    qf.readline()

    def qmp(cmd, args):
        qf.write(json.dumps({"execute": cmd, "arguments": args}).encode() + b"\n")
        qf.flush()
        while True:  # skip events until the command's own reply
            line = json.loads(qf.readline())
            if "return" in line or "error" in line:
                if "error" in line:
                    print("usb-mouse: QMP", cmd, "error:", line["error"], file=sys.stderr)
                return line

    def mouse(events):
        return qmp("input-send-event", {"events": events})

    rx, ry = dx, dy
    while rx or ry:
        sx = max(-40, min(40, rx))
        sy = max(-40, min(40, ry))
        mouse([{"type": "rel", "data": {"axis": "x", "value": sx}},
               {"type": "rel", "data": {"axis": "y", "value": sy}}])
        rx, ry = rx - sx, ry - sy
        drain(0.1)
    drain(0.5)
    for _ in range(clicks):
        mouse([{"type": "btn", "data": {"down": True, "button": "left"}}])
        drain(0.1)
        mouse([{"type": "btn", "data": {"down": False, "button": "left"}}])
        drain(0.1)
    drain(0.5)
    for way in os.environ.get("MOUSE_WHEEL", "").split():
        btn = "wheel-up" if way == "up" else "wheel-down"
        mouse([{"type": "btn", "data": {"down": True, "button": btn}}])
        drain(0.1)
        mouse([{"type": "btn", "data": {"down": False, "button": btn}}])
        drain(0.3)
    drain(0.5)
    for text in texts:
        for ch in text + "\n":
            code = "ret" if ch == "\n" else QCODE.get(ch, ch)
            qmp("send-key", {"keys": [{"type": "qcode", "data": code}], "hold-time": 40})
            drain(0.15)
        drain(4)
    drain(1)
    qmp("screendump", {"filename": png, "format": "png"})
    drain(1)
    open(out, "wb").write(bytes(cap))


if __name__ == "__main__":
    main()
