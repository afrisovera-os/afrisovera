#!/usr/bin/env python3
"""Type on QEMU's emulated USB keyboard and capture what the guest's serial console says.

    usb-type.py <serial port> <qmp port> <out file> <text>...

Connects to the guest's serial console (tcp, QEMU `server,nowait`) and its QMP socket,
waits for the shell prompt on the serial console, then presses each character of every
<text> on the USB keyboard through QMP `send-key` (Enter after each), and writes the whole
serial capture to <out file>. Nothing is ever typed on the serial line itself: whatever the
shell answers to, it read from the USB keyboard. Used by scripts/pc-usbkbd-test.sh and pc-ps2kbd-test.sh (with no USB keyboard, QEMU sends the
keys to the PS/2 one). USB_TYPE_SHOT=<png> also saves a screenshot at the end.
"""
import json
import os
import socket
import sys
import time

QCODE = {" ": "spc", "-": "minus", "=": "equal", ".": "dot", ",": "comma", "/": "slash",
         ";": "semicolon", "'": "apostrophe", "[": "bracket_left", "]": "bracket_right",
         "\\": "backslash", "`": "grave_accent", "\n": "ret"}
SHIFTED = {"_": "minus", "+": "equal", ":": "semicolon", "?": "slash", "!": "1", "@": "2",
           "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9", ")": "0"}


def connect(port, deadline):
    while time.time() < deadline:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=2)
        except OSError:
            time.sleep(0.3)
    raise SystemExit(f"usb-type: nothing listening on {port}")


def main():
    sport, qport, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    texts = sys.argv[4:]
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
        raise SystemExit("usb-type: the shell prompt never appeared")
    drain(2)
    q = connect(qport, deadline)
    qf = q.makefile("rwb")
    qf.readline()  # greeting
    qf.write(b'{"execute":"qmp_capabilities"}\n')
    qf.flush()
    qf.readline()

    def key(*codes):
        keys = [{"type": "qcode", "data": c} for c in codes]
        qf.write(json.dumps({"execute": "send-key", "arguments": {"keys": keys, "hold-time": 40}}).encode() + b"\n")
        qf.flush()
        while True:  # skip events until the command's own reply
            line = json.loads(qf.readline())
            if "return" in line or "error" in line:
                return line

    for text in texts:
        for ch in text + "\n":
            if ch in SHIFTED:
                key("shift", SHIFTED[ch])
            elif ch.isalpha() and ch.isupper():
                key("shift", ch.lower())
            else:
                key(QCODE.get(ch, ch))
            drain(0.15)
        drain(4)
    drain(3)
    shot = os.environ.get("USB_TYPE_SHOT")  # optional: a screenshot of the guest's screen at the end
    if shot:
        qf.write(json.dumps({"execute": "screendump", "arguments": {"filename": shot, "format": "png"}}).encode() + b"\n")
        qf.flush()
        qf.readline()
    open(out, "wb").write(bytes(cap))


if __name__ == "__main__":
    main()
