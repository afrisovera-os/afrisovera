#!/usr/bin/env python3
"""Capture the QEMU display via QMP. Usage: qmp-screendump.py <host:port> <out.png>"""
import json
import socket
import sys


def main():
    hostport = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:4444"
    out = sys.argv[2] if len(sys.argv) > 2 else "fb.png"
    host, port = hostport.split(":")

    s = socket.create_connection((host, int(port)), timeout=10)
    f = s.makefile("rwb")

    def send(obj):
        f.write((json.dumps(obj) + "\n").encode())
        f.flush()

    def recv():
        return json.loads(f.readline().decode())

    recv()  # QMP greeting
    send({"execute": "qmp_capabilities"})
    recv()
    # QEMU >= 7.1 supports an explicit format arg; fall back to default (PPM) if not.
    send({"execute": "screendump", "arguments": {"filename": out, "format": "png"}})
    resp = recv()
    if "error" in resp:
        send({"execute": "screendump", "arguments": {"filename": out}})
        resp = recv()
    print(json.dumps(resp))
    s.close()
    sys.exit(0 if "return" in resp else 1)


if __name__ == "__main__":
    main()
