#!/usr/bin/env python3
"""Phase 2.776 — a stand-in for `qemu-system-aarch64` that speaks a TCP SERIAL
socket AND a QMP socket, and that keeps a GUEST CLOCK, so `shell-wfi-test.sh`'s
`sleep 6` can be falsified with ZERO boots.

WHY A THIRD REPLAY GUEST
------------------------
`replay-guest-2771.py` writes stdout (for the `-serial stdio` dialect:
`pi`/`dhcp`/`config`). `replay-serial-2775.py` is a serial PEER and can RESET,
for `reboot`, whose client is an inline python that only reads. Neither can
serve `shell-wfi`, because that suite drives `scripts/shell-demo.py`, which

  * connects to the QMP port and performs a screendump — a replay guest that
    never answers there hangs the driver at `qf.readline()` (the failure mode
    2.772 measured on the real driver, so it must not be simulated by accident);
  * types COMMANDS and waits for each answer to end in a prompt; and
  * asserts on two numbers the guest computes from ITS OWN clock — `uptime`'s
    tick count and `irqstat`'s `timer N ticks, uart rx R, idle-wfi W`.

That last one is the whole point. The red this fix exists for is `idle-wfi 0`
at `timer 12 ticks` (`.rat-logs-2770/shell-wfi.out`) against a green run's
`idle-wfi 739` at `timer 749` (`.rat-logs-2769/shell-wfi.out`) — a GUEST that
got ~120 ms of CPU inside a 6 s HOST sleep. Reproducing that needs a peer whose
clock can be told to run slower than the host's, which is what this is.

WHAT IT IS
----------
A pacer with a two-dial clock, not a model of QEMU or of the kernel. It proves
nothing about either. It can be trusted for exactly one claim: "with the guest
advancing at this rate and its idle loop counting at this rate, this is what the
suite reads." Arithmetic on clocks.

THE TWO DIALS, AND WHY THEY ARE SEPARATE
  REPLAY_TICK_HZ         guest ticks per HOST second. 100 = a guest getting a
                         whole CPU (the kernel's timer is 100 Hz); 4 = the walk's
                         row 34, which reached 12 ticks in ~3 s of host time.
  REPLAY_WFI_PER_TICK    idle-wfi accrued per idle tick. The green capture
                         measures 739/749 = 0.99, i.e. one `wfi` per timer tick,
                         because each sleeps until the next timer IRQ. 0 = a
                         console that never idles — the defect this suite exists
                         to catch.
In the real kernel these are NOT independent: a descheduled guest does not run
`cpu_getc`, so starvation forces the second to zero as well — which is exactly
what row 34 recorded. The dials are separate here so that the falsifier can put
a REAL console defect (fast guest, zero wfi) and a STARVED guest (slow guest,
zero wfi) in front of the same suite and see two different verdicts. Collapsing
them into one would make the suite unable to fail for the reason it exists.

ENV (not flags — flags would collide with the qemu argv it must accept)
  REPLAY_LOG           capture to replay as the boot text        (required)
  REPLAY_PACE_MS       ms between boot lines                     (default 0)
  REPLAY_TICK_HZ       guest ticks per host second               (default 100)
  REPLAY_WFI_PER_TICK  idle-wfi per idle tick                    (default 1.0)
  REPLAY_HOLD_S        seconds to serve the shell after boot     (default 600)
  REPLAY_NO_QMP        1 = bind QMP but never answer it          (default off)
"""
import os
import re
import socket
import sys
import threading
import time

CAP = os.environ.get("REPLAY_LOG")
if not CAP:
    sys.stderr.write("replay-wfi: REPLAY_LOG is unset\n")
    sys.exit(2)

PACE = float(os.environ.get("REPLAY_PACE_MS", "0")) / 1000.0
TICK_HZ = float(os.environ.get("REPLAY_TICK_HZ", "100"))
WFI_PER_TICK = float(os.environ.get("REPLAY_WFI_PER_TICK", "1.0"))
HOLD = float(os.environ.get("REPLAY_HOLD_S", "600"))
NO_QMP = os.environ.get("REPLAY_NO_QMP", "") not in ("", "0")

with open(CAP, "r", encoding="utf-8", errors="replace") as fh:
    LINES = fh.read().splitlines()

TCP_RE = re.compile(r"tcp:([\d.]+):(\d+)")
PROMPT = b"afrisovera> "


def find_port(argv, flag):
    for i, a in enumerate(argv):
        if a == flag and i + 1 < len(argv):
            m = TCP_RE.search(argv[i + 1])
            if m:
                return m.group(1), int(m.group(2))
    return None


class GuestClock:
    """Guest time, advanced lazily from the host clock at TICK_HZ.

    `ticks` is what `uptime` and `irqstat` print; `idle_wfi` accrues only across
    intervals where the guest was IDLE (no command in flight), which is the
    kernel's own rule: `note_idle_wfi()` has one call site, in `cpu_getc`, on
    the branch where the RX ring came back empty (kernel/src/shell.rs:69).
    """

    def __init__(self):
        self.t0 = time.time()
        self.last = self.t0
        self.ticks = 0.0
        self.idle_wfi = 0.0
        self.rx_irqs = 0

    def advance(self, idle=True):
        now = time.time()
        d = (now - self.last) * TICK_HZ
        self.last = now
        self.ticks += d
        if idle:
            self.idle_wfi += d * WFI_PER_TICK


clock = GuestClock()

ser_addr = find_port(sys.argv, "-serial")
qmp_addr = find_port(sys.argv, "-qmp")
if ser_addr is None:
    ser_addr = (os.environ.get("REPLAY_HOST", "127.0.0.1"),
                int(os.environ.get("REPLAY_PORT", "0")))

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(ser_addr)
srv.listen(1)


def qmp_server(addr):
    """Answer the driver's three lines: greeting, capabilities, screendump.

    QEMU sends the greeting UNPROMPTED on accept and the driver's first act is
    `qf.readline()`, so the greeting has to precede any request. Everything else
    is `{"return": {}}`, which is what the real screendump returns (the driver
    prints it verbatim: `screendump: {"return": {}}` in every green capture).
    """
    q = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    q.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        q.bind(addr)
        q.listen(1)
    except OSError as e:
        sys.stderr.write("replay-wfi: QMP bind failed: %s\n" % e)
        return
    while True:
        try:
            c, _ = q.accept()
        except OSError:
            return
        if NO_QMP:
            continue                                  # bound, deaf: 2.772's shape
        try:
            c.sendall(b'{"QMP": {"version": {"qemu": {"micro": 0, "minor": 2,'
                      b' "major": 8}, "package": "replay-wfi"}, "capabilities": []}}\n')
            f = c.makefile("rwb")
            for line in f:
                if not line.strip():
                    continue
                f.write(b'{"return": {}}\n')
                f.flush()
        except OSError:
            pass
        finally:
            try:
                c.close()
            except OSError:
                pass


if qmp_addr is not None:
    threading.Thread(target=qmp_server, args=(qmp_addr,), daemon=True).start()

sys.stderr.write("replay-wfi: serial %s:%d qmp %s (pace %.0f ms/line,"
                 " %.0f ticks/s, %.2f wfi/tick)\n"
                 % (ser_addr[0], ser_addr[1],
                    "%s:%d" % qmp_addr if qmp_addr else "none",
                    PACE * 1000, TICK_HZ, WFI_PER_TICK))
sys.stderr.flush()

conn, _peer = srv.accept()
conn.settimeout(0.2)


def answer(cmd):
    """The three commands this suite drives, printed the way AfriSh prints them.

    Formats are copied from the kernel, not invented: `uptime` is
    kernel/src/shell.rs:22085-22086 (`{} ticks  (~{} ms, ~{} s)`, two spaces),
    `irqstat` is shell.rs:22209-22214 (`irqs: timer {} ticks, uart rx {},
    idle-wfi {}`), `echo` returns its argument. An unknown command answers
    nothing and still prints a prompt, as the shell does.
    """
    t = int(clock.ticks)
    if cmd == "uptime":
        return "%d ticks  (~%d ms, ~%d s)" % (t, t * 10, t // 100)
    if cmd == "irqstat":
        return "irqs: timer %d ticks, uart rx %d, idle-wfi %d" % (
            t, clock.rx_irqs, int(clock.idle_wfi))
    if cmd.startswith("echo "):
        return cmd[5:]
    return None


try:
    for ln in LINES:                                   # the boot
        conn.sendall(ln.encode("utf-8", "replace") + b"\r\n")
        if PACE:
            time.sleep(PACE)
    conn.sendall(b"\r\n" + PROMPT)
    clock.advance(idle=True)

    deadline = time.time() + HOLD
    typed = bytearray()
    while time.time() < deadline:
        try:
            b = conn.recv(4096)
        except socket.timeout:
            clock.advance(idle=True)                   # silence: the shell wfi's
            continue
        except OSError:
            break
        if not b:
            break
        clock.advance(idle=True)                       # idle UP TO this keystroke
        clock.rx_irqs += 1
        conn.sendall(b)                                # the shell echoes
        typed.extend(b)
        while b"\r" in typed or b"\n" in typed:
            i = min(x for x in (typed.find(b"\r"), typed.find(b"\n")) if x >= 0)
            line = bytes(typed[:i]).decode("latin1").strip()
            del typed[:i + 1]
            clock.advance(idle=False)                  # running a command != idle
            out = answer(line)
            conn.sendall(b"\r\n" + ((out.encode() + b"\r\n") if out else b"") + PROMPT)
except (OSError, BrokenPipeError):
    pass
finally:
    try:
        conn.close()
    except OSError:
        pass
    srv.close()
