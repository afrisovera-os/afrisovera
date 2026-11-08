#!/usr/bin/env python3
"""Phase 2.775 — a stand-in for `qemu-system-aarch64` that speaks a TCP SERIAL
socket and can RESET, so `reboot-test.sh`'s `drain(9)` window can be falsified
with ZERO boots.

WHY A SECOND REPLAY GUEST
-------------------------
scripts/replay-guest-2771.py already replays a real capture, but it writes to
STDOUT, because the three suites it serves (`pi`/`dhcp`/`config`) use
`-serial stdio` and grep a file. `reboot` is the other dialect: it hands QEMU
`-serial tcp:127.0.0.1:4662,server,nowait` and drives the guest from an inline
python CLIENT over that socket. A stdout pacer cannot be a peer for a client.

Worse, `reboot` is the only suite in the registry whose guest RESTARTS: the
whole point of the test is that PSCI SYSTEM_RESET re-runs the boot path, so the
banner appears a SECOND time on the SAME socket. Nothing in the tree could
stand in for that, so the two arms that matter — "slow but correct guest" and
"guest that acknowledges the command but never resets" — had no way to be
reached without booting. The 684-suite walk owns this box (a contending guest
reds rows for no kernel reason, which is the exact defect under audit), so the
guest is replaced rather than the harness.

WHAT IT IS
----------
A pacer with one state machine, not a model of QEMU. It proves nothing about
the kernel. It can be trusted for exactly one claim: "with the boot text
arriving at this rate, this is what a client holding the socket has read by the
time its wait ends" — arithmetic on bytes and clocks.

  * accepts and IGNORES arbitrary qemu-style argv, so it drops into the suite's
    own command line unchanged (the suite is not edited to be tested);
  * reads `-serial tcp:HOST:PORT,server...` out of that argv and binds it, so a
    PATH shim named `qemu-system-aarch64` makes the REAL suite drive this;
  * binds `-qmp tcp:HOST:PORT,server` too and never speaks on it — `reboot`'s
    client never connects, but a silent refusal there would be a difference
    between this peer and QEMU that a later suite could trip over;
  * replays a REAL capture (.rat-logs-2770/mansa-smp.log, 91 lines, this walk's
    own row-2 boot) at REPLAY_PACE_MS per line;
  * then reads the client. On a line containing `reboot` it echoes the typed
    bytes (the shell does), prints the kernel's own
    `[reboot] resetting via PSCI SYSTEM_RESET...` (kernel/src/shell.rs:20834,
    verbatim) and REPLAYS THE CAPTURE AGAIN — that is the reset;
  * or, with REPLAY_NO_RESET=1, prints the acknowledgement and never resets.
    That is the genuinely-broken kernel this suite exists to catch, and it is
    the arm that decides whether a ceiling swallows a real fault.

ENV (not flags — flags would collide with the qemu argv it must accept)
  REPLAY_LOG        capture to replay for EACH boot        (required)
  REPLAY_PACE_MS    ms between lines                       (default 0)
  REPLAY_NO_RESET   1 = acknowledge `reboot`, never reset  (default off)
  REPLAY_HOLD_S     seconds to idle after the last boot    (default 3600)
  REPLAY_EOF_AFTER  seconds after the client connects, then CLOSE the socket
                    and exit. Stands in for a guest that dies mid-wait; it is
                    how the fix is checked for the failure mode it could
                    introduce — a ceiling that spins on a dead socket for 90s
                    where the old 9s window would have given up at 9.
  REPLAY_PORTFILE   write the actually-bound serial port here (for port 0)
"""
import os
import re
import socket
import sys
import threading
import time

CAP = os.environ.get("REPLAY_LOG")
if not CAP:
    sys.stderr.write("replay-serial: REPLAY_LOG is unset\n")
    sys.exit(2)

PACE = float(os.environ.get("REPLAY_PACE_MS", "0")) / 1000.0
NO_RESET = os.environ.get("REPLAY_NO_RESET", "") not in ("", "0")
HOLD = float(os.environ.get("REPLAY_HOLD_S", "3600"))
EOF_AFTER = float(os.environ.get("REPLAY_EOF_AFTER", "0") or 0)
PORTFILE = os.environ.get("REPLAY_PORTFILE")

with open(CAP, "r", encoding="utf-8", errors="replace") as fh:
    LINES = fh.read().splitlines()

# `-serial tcp:127.0.0.1:4662,server,nowait` / `-qmp tcp:127.0.0.1:4663,...`
TCP_RE = re.compile(r"tcp:([\d.]+):(\d+)")


def find_port(argv, flag):
    for i, a in enumerate(argv):
        if a == flag and i + 1 < len(argv):
            m = TCP_RE.search(argv[i + 1])
            if m:
                return m.group(1), int(m.group(2))
    return None


ser_addr = find_port(sys.argv, "-serial")
qmp_addr = find_port(sys.argv, "-qmp")
if ser_addr is None:
    host = os.environ.get("REPLAY_HOST", "127.0.0.1")
    ser_addr = (host, int(os.environ.get("REPLAY_PORT", "0")))

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(ser_addr)
srv.listen(1)
bound_port = srv.getsockname()[1]
if PORTFILE:
    # Written only after bind(), so a falsifier that waits for this file cannot
    # race the listen(). Picking a free port in the parent and passing it down
    # would have that race; this does not.
    with open(PORTFILE, "w", encoding="utf-8") as fh:
        fh.write(str(bound_port) + "\n")

qmp_srv = None
if qmp_addr is not None:
    try:
        qmp_srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        qmp_srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        qmp_srv.bind(qmp_addr)
        qmp_srv.listen(1)
    except OSError:
        qmp_srv = None

sys.stderr.write(f"replay-serial: listening on {ser_addr[0]}:{bound_port}"
                 f" (pace {PACE * 1000:.0f} ms/line, reset={'no' if NO_RESET else 'yes'})\n")
sys.stderr.flush()

conn, _peer = srv.accept()
conn.settimeout(0.2)


def replay(sock):
    """One boot: the capture, line by line, at PACE. QEMU's serial is CRLF."""
    for ln in LINES:
        sock.sendall(ln.encode("utf-8", "replace") + b"\r\n")
        if PACE:
            time.sleep(PACE)


if EOF_AFTER:
    # Deliberately a timer and not a line count: the arm is about the CLIENT's
    # wait meeting a dead socket, so the death has to be able to land in the
    # middle of a wait rather than at a fixed point in the text.
    def _die():
        time.sleep(EOF_AFTER)
        try:
            conn.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        conn.close()
        os._exit(0)

    threading.Thread(target=_die, daemon=True).start()

try:
    replay(conn)                                   # boot 1
    deadline = time.time() + HOLD
    typed = bytearray()
    while time.time() < deadline:
        try:
            b = conn.recv(4096)
        except socket.timeout:
            continue
        except OSError:
            break
        if not b:
            break
        conn.sendall(b)                            # the shell echoes what you type
        typed.extend(b)
        if b"reboot" in typed and (b"\r" in typed or b"\n" in typed):
            typed.clear()
            conn.sendall(b"\r\n[reboot] resetting via PSCI SYSTEM_RESET...\r\n")
            if not NO_RESET:
                replay(conn)                       # boot 2 — the reset
    time.sleep(0.2)
except (OSError, BrokenPipeError):
    pass
finally:
    try:
        conn.close()
    except OSError:
        pass
    srv.close()
    if qmp_srv is not None:
        qmp_srv.close()
