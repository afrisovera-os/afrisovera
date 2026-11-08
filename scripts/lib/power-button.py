#!/usr/bin/env python3
"""Type commands on the guest's serial shell, then press the PC's power button through QMP.

    power-button.py <serial port> <qmp port> <out file> <cmd>...

Connects to the guest's serial console (tcp, QEMU `server,nowait`), waits for the shell
prompt, types every <cmd> (Enter after each, waiting for the next prompt), then sends QMP
`system_powerdown` -- QEMU's short press of the ACPI power button (PWRBTN_STS, only latched
once the guest has set PWRBTN_EN). It then keeps reading the serial line until QEMU closes it
(the guest switched the machine off itself) or 60 s pass. The capture is written to
<out file>, followed by one line the grader reads:
    == power-button: qemu exited on its own after N s ==   or
    == power-button: qemu still running after 60 s ==
Used by scripts/pc-powerbutton-test.sh.
"""
import json
import socket
import sys
import time


def connect(port, deadline):
    while time.time() < deadline:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=2)
        except OSError:
            time.sleep(0.3)
    raise SystemExit(f"power-button: nothing listening on {port}")


def main():
    sport, qport, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    cmds = sys.argv[4:]
    deadline = time.time() + 240
    ser = connect(sport, deadline)
    ser.settimeout(0.5)
    cap = bytearray()
    closed = [False]

    def drain(seconds, until=None, start=0):
        end = time.time() + seconds
        while time.time() < end:
            try:
                b = ser.recv(4096)
                if not b:
                    closed[0] = True
                    return False
                cap.extend(b)
                if until and until in cap[start:]:
                    return True
            except socket.timeout:
                pass
            except OSError:
                closed[0] = True
                return False
        return False

    if not drain(220, b"afrisovera> "):
        open(out, "wb").write(bytes(cap) + b"\n== power-button: the shell prompt never appeared ==\n")
        raise SystemExit("power-button: the shell prompt never appeared")
    drain(2)
    for c in cmds:
        mark = len(cap)
        ser.sendall(c.encode() + b"\r")
        drain(20, b"afrisovera> ", mark + len(c))
        drain(1)
    q = connect(qport, deadline)
    qf = q.makefile("rwb")
    qf.readline()  # greeting
    qf.write(b'{"execute":"qmp_capabilities"}\n')
    qf.flush()
    qf.readline()
    qf.write(b'{"execute":"system_powerdown"}\n')
    qf.flush()
    t0 = time.time()
    while True:  # skip events until the command's own reply
        line = qf.readline()
        if not line:
            break
        msg = json.loads(line)
        if "return" in msg or "error" in msg:
            if "error" in msg:
                print("power-button: QMP error:", msg["error"], file=sys.stderr)
            break
    while not closed[0] and time.time() - t0 < 60:
        drain(1)
    if closed[0]:
        tail = f"\n== power-button: qemu exited on its own after {int(time.time() - t0)} s ==\n"
    else:
        tail = "\n== power-button: qemu still running after 60 s ==\n"
    open(out, "wb").write(bytes(cap) + tail.encode())


if __name__ == "__main__":
    main()
