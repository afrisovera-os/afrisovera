#!/usr/bin/env python3
"""Drive QEMU's USB keyboard and mouse through QMP, step by step, and capture the guest's
serial console.

    usb-steps.py <serial port> <qmp port> <out file> <png> <step>...

Steps (in order):  type:<text>   keys on the USB keyboard, then Enter
                   move:<dx>,<dy> relative mouse move (sent in pieces of at most 40)
                   abs:<x>,<y>   absolute pointer position, 0..32767 each (a usb-tablet)
                   down / up     left mouse button
                   hold:<key>,<ms>  hold one key down that long (key repeat)
                   plug:<driver>,<id>[,<xhci id>[,<port>]] / unplug:<id>  hot-plug a USB device (QMP device_add/del)
                   wait:<s>      let the guest catch up
The capture starts at the shell prompt; a screenshot is saved to <png> at the end. Nothing is
typed on the serial line. Used by scripts/pc-wmdrag-test.sh.
"""
import json
import socket
import sys
import time

QCODE = {" ": "spc", "-": "minus", ".": "dot", ",": "comma", "/": "slash", "_": "minus"}


def connect(port, deadline):
    while time.time() < deadline:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=2)
        except OSError:
            time.sleep(0.3)
    raise SystemExit(f"usb-steps: nothing listening on {port}")


def main():
    sport, qport, out, png = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4]
    steps = sys.argv[5:]
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
        raise SystemExit("usb-steps: the shell prompt never appeared")
    drain(2)
    q = connect(qport, deadline)
    qf = q.makefile("rwb")
    qf.readline()
    qf.write(b'{"execute":"qmp_capabilities"}\n')
    qf.flush()
    qf.readline()

    def qmp(cmd, args):
        qf.write(json.dumps({"execute": cmd, "arguments": args}).encode() + b"\n")
        qf.flush()
        while True:
            line = json.loads(qf.readline())
            if "return" in line or "error" in line:
                if "error" in line:
                    print("usb-steps: QMP", cmd, "error:", line["error"], file=sys.stderr)
                return line

    for st in steps:
        kind, _, arg = st.partition(":")
        if kind == "type":
            for ch in arg + "\n":
                code = "ret" if ch == "\n" else QCODE.get(ch, ch)
                keys = [{"type": "qcode", "data": code}]
                if ch == "_":
                    keys.insert(0, {"type": "qcode", "data": "shift"})
                qmp("send-key", {"keys": keys, "hold-time": 40})
                drain(0.15)
            drain(4)
        elif kind == "move":
            rx, ry = (int(v) for v in arg.split(","))
            while rx or ry:
                sx, sy = max(-40, min(40, rx)), max(-40, min(40, ry))
                qmp("input-send-event", {"events": [{"type": "rel", "data": {"axis": "x", "value": sx}},
                                                    {"type": "rel", "data": {"axis": "y", "value": sy}}]})
                rx, ry = rx - sx, ry - sy
                drain(0.1)
            drain(0.5)
        elif kind == "abs":
            ax, ay = (int(v) for v in arg.split(","))
            qmp("input-send-event", {"events": [{"type": "abs", "data": {"axis": "x", "value": ax}},
                                                {"type": "abs", "data": {"axis": "y", "value": ay}}]})
            drain(0.5)
        elif kind in ("down", "up"):
            qmp("input-send-event", {"events": [{"type": "btn", "data": {"down": kind == "down", "button": "left"}}]})
            drain(0.4)
        elif kind == "hold":
            key, ms = arg.split(",")
            qmp("send-key", {"keys": [{"type": "qcode", "data": key}], "hold-time": int(ms)})
            drain(int(ms) / 1000 + 0.5)
        elif kind == "plug":  # plug:<driver>,<id>[,<xhci id>[,<port>]]  e.g. plug:usb-kbd,k1 (default xHCI "xhci")
            drv, did, *ctl = arg.split(",")  # <port> is QEMU's: "1.2" = port 2 of the hub on port 1
            dev = {"driver": drv, "id": did, "bus": (ctl[0] if ctl else "xhci") + ".0"}
            if len(ctl) > 1:
                dev["port"] = ctl[1]
            qmp("device_add", dev)
            drain(3)
        elif kind == "unplug":  # unplug:<id>
            qmp("device_del", {"id": arg})
            drain(3)
        elif kind == "wait":
            drain(float(arg))
        else:
            raise SystemExit("usb-steps: unknown step " + st)
    drain(1)
    qmp("screendump", {"filename": png, "format": "png"})
    drain(1)
    open(out, "wb").write(bytes(cap))


if __name__ == "__main__":
    main()
