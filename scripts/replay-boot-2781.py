#!/usr/bin/env python3
"""Phase 2.781 — a stand-in guest for falsifying `shell-demo.py`'s BOOT_QUIET
fall-through (`shell-demo.py:54`, the driver window ruled on `rjust`'s 185 s red
in the 2.770 walk) with ZERO real boots.

WHY A FOURTH REPLAY GUEST
-------------------------
`replay-guest-2771.py` writes stdout. `replay-serial-2775.py` is a serial peer
that can RESET. `replay-wfi-2776.py` keeps a two-dial GUEST CLOCK so a suite can
read `irqstat`. None of the three can serve this subject, because the subject is
not a suite's arithmetic — it is the driver's decision at the *attach* boundary,
and that decision reads exactly two things neither existing guest can vary:

  * WHEN the first boot byte arrives after the client attaches (`REPLAY_SILENT_S`
    and `REPLAY_PRE_ATTACH`), which is the ambiguity `wait_ready` resolves; and
  * whether the guest SURVIVES being typed at mid-boot (`REPLAY_WEDGE`), which is
    the second half of the mechanism and the reason the fall-through is not free.

THE WEDGE IS NOT INVENTED HERE. `shell-demo.py:170-178` records it as measured on
the real kernel: "a version of this poll that nudged with a bare CR was measured
to WEDGE the guest ... (0 bytes -> prompt at 14.4s; 1 byte -> never, boot dies at
the preemption demo)". `.rat-logs-2770/rjust.out` is that shape on a real walk:
36 lines that stop dead on `[mansa] preemption demo: 2 CPU-bound EL0 workers`,
with `AfriSh ready` appearing zero times. This guest replays that ending; it does
not explain it. Why one byte kills a booting kernel is still unmeasured and is
its own milestone.

WHAT IT IS, AND WHAT IT IS NOT
------------------------------
A pacer with three switches. It is NOT a model of QEMU or of the kernel and
proves nothing about either. It can be trusted for exactly one claim: "with the
guest going quiet for this long and dying on an early keystroke like this, THIS
is what the driver does" — the driver's control flow, not the guest's physics.

ENV (not flags — flags would collide with the qemu argv it must be able to accept)
  REPLAY_LOG        capture whose guest lines are the boot text      (required)
  REPLAY_SILENT_S   seconds of silence AFTER accept, before byte 1   (default 0)
  REPLAY_PACE_MS    ms between boot lines                            (default 0)
  REPLAY_WEDGE      1 = a byte arriving before the boot finishes stops the boot
                    replay for good, at the WEDGE_AT line            (default 0)
  REPLAY_WEDGE_AT   substring naming the line the wedged boot ends on
                                                (default "preemption demo")
  REPLAY_PRE_ATTACH 1 = the boot happened BEFORE this client attached and QEMU
                    does not replay it: send NOTHING, but answer commands. This
                    is the case the fall-through exists for.         (default 0)
  REPLAY_HOLD_S     seconds to serve the shell after boot            (default 120)
  REPLAY_NO_QMP     1 = bind QMP but never answer it                 (default off)
"""
import os
import re
import socket
import sys
import threading
import time

CAP = os.environ.get("REPLAY_LOG")
if not CAP:
    sys.stderr.write("replay-boot: REPLAY_LOG is unset\n")
    sys.exit(2)

SILENT = float(os.environ.get("REPLAY_SILENT_S", "0"))
PACE = float(os.environ.get("REPLAY_PACE_MS", "0")) / 1000.0
WEDGE = os.environ.get("REPLAY_WEDGE", "") not in ("", "0")
WEDGE_AT = os.environ.get("REPLAY_WEDGE_AT", "preemption demo")
PRE_ATTACH = os.environ.get("REPLAY_PRE_ATTACH", "") not in ("", "0")
HOLD = float(os.environ.get("REPLAY_HOLD_S", "120"))
NO_QMP = os.environ.get("REPLAY_NO_QMP", "") not in ("", "0")

with open(CAP, "r", encoding="utf-8", errors="replace") as fh:
    RAW = fh.read().splitlines()

# The driver writes its own `screendump: ...` line into the capture; the guest
# never sent it. Replaying it would put a driver artefact on the wire.
GUEST_LINES = [ln for ln in RAW if not ln.startswith("screendump:")]

# THE BOOT IS NOT THE CAPTURE. A suite's capture holds the boot AND the shell
# session that followed it — prompts, typed commands, answers. Replaying all of
# it hands the driver a prompt in the MIDDLE of the replay: `wait_ready` returns
# on the first `afrisovera>` it sees and starts typing while this script is still
# feeding the rest of the file, so every arm silently becomes a "typed mid-boot"
# arm and the control arms stop being controls. Measured, not reasoned: with the
# whole 46-line file as boot text, arm B came back `prompt=1 banner=1 answers=0`
# — green on both boot markers and deaf to all three commands.
# So the boot text stops at the banner, and the prompt after it is synthesised.
BOOT_ENDS = os.environ.get("REPLAY_BOOT_ENDS", "AfriSh ready")
LINES = GUEST_LINES
for _i, _ln in enumerate(GUEST_LINES):
    if BOOT_ENDS in _ln:
        LINES = GUEST_LINES[:_i + 1]
        break

TCP_RE = re.compile(r"tcp:([\d.]+):(\d+)")
PROMPT = b"afrisovera> "


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
    ser_addr = (os.environ.get("REPLAY_HOST", "127.0.0.1"),
                int(os.environ.get("REPLAY_PORT", "0")))
if qmp_addr is None and os.environ.get("REPLAY_QMP_PORT"):
    qmp_addr = (ser_addr[0], int(os.environ["REPLAY_QMP_PORT"]))

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(ser_addr)
srv.listen(1)


def qmp_server(addr):
    """Answer the driver's three lines: greeting, capabilities, screendump.

    QEMU sends the greeting UNPROMPTED on accept and the driver's first act is
    `qf.readline()`, so the greeting has to precede any request. Everything else
    is `{"return": {}}`, which is what the real screendump returns.
    """
    q = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    q.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        q.bind(addr)
        q.listen(1)
    except OSError as e:
        sys.stderr.write("replay-boot: QMP bind failed: %s\n" % e)
        return
    while True:
        try:
            c, _ = q.accept()
        except OSError:
            return
        if NO_QMP:
            continue                                   # bound, deaf: 2.772's shape
        try:
            c.sendall(b'{"QMP": {"version": {"qemu": {"micro": 0, "minor": 2,'
                      b' "major": 8}, "package": "replay-boot"}, "capabilities": []}}\n')
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

sys.stderr.write("replay-boot: serial %s:%d qmp %s (silent %.1fs, pace %.0f ms/line,"
                 " wedge %s at %r, pre-attach %s)\n"
                 % (ser_addr[0], ser_addr[1],
                    "%s:%d" % qmp_addr if qmp_addr else "none",
                    SILENT, PACE * 1000, "on" if WEDGE else "off", WEDGE_AT,
                    "yes" if PRE_ATTACH else "no"))
sys.stderr.flush()

conn, _peer = srv.accept()
conn.settimeout(0.05)

typed_early = threading.Event()
boot_done = threading.Event()
wedged_ev = threading.Event()
typed = bytearray()
lock = threading.Lock()


def reader():
    """Read the client's keystrokes for the WHOLE run.

    It has to run alongside the boot replay, because the defect being falsified
    is precisely that the driver types DURING the boot. A guest that only starts
    listening once its banner is out cannot be typed at too early, which would
    make the mechanism unreachable.
    """
    while not stop.is_set():
        try:
            b = conn.recv(4096)
        except socket.timeout:
            continue
        except OSError:
            return
        if not b:
            return
        if not boot_done.is_set():
            typed_early.set()
        with lock:
            typed.extend(b)


stop = threading.Event()
threading.Thread(target=reader, daemon=True).start()


def answer(cmd):
    """A generic shell answer. The subject here is the DRIVER's control flow, so
    the guest only has to end every reply in a prompt the way AfriSh does; what
    the string-utils commands compute is graded by their own 684 suites."""
    if not cmd:
        return None
    if cmd.startswith("echo "):
        return cmd[5:]
    return "%s: ok" % cmd.split()[0]


try:
    if PRE_ATTACH:
        # The boot happened with no client attached. QEMU does not replay it, so
        # this connection sees TOTAL SILENCE until it is spoken to. The guest is
        # nevertheless at the prompt and perfectly healthy.
        boot_done.set()
    else:
        if SILENT:
            # Silence between accept and byte 1: the starved guest that has not
            # got to its first print yet. Indistinguishable, on this socket, from
            # the PRE_ATTACH case above — which is the whole ambiguity.
            end = time.time() + SILENT
            while time.time() < end and not stop.is_set():
                time.sleep(0.02)
        for ln in LINES:
            if WEDGE and typed_early.is_set():
                conn.sendall(ln.encode("utf-8", "replace") + b"\r\n")
                if WEDGE_AT in ln:
                    # Dead here, for good: `.rat-logs-2770/rjust.out`'s ending.
                    sys.stderr.write("replay-boot: WEDGED at %r (typed at mid-boot)\n" % ln[:48])
                    sys.stderr.flush()
                    wedged_ev.set()
                    boot_done.set()                    # never prompts; still reads
                    break
                continue
            conn.sendall(ln.encode("utf-8", "replace") + b"\r\n")
            if PACE:
                time.sleep(PACE)
        else:
            conn.sendall(b"\r\n" + PROMPT)
            boot_done.set()

    deadline = time.time() + HOLD
    # Only an ACTUAL wedge, not merely "someone typed early". Conflating the two
    # made arm B answer nothing: the wedge line had already gone past, the boot
    # finished normally, and the guest still refused to serve the shell.
    wedged = wedged_ev.is_set()
    while time.time() < deadline:
        if wedged:
            # A wedged guest still has a live socket — it simply never answers.
            # Draining without replying is what makes the driver burn CMD_QUIET
            # once per command instead of failing fast.
            time.sleep(0.05)
            continue
        with lock:
            has = b"\r" in typed or b"\n" in typed
            if has:
                i = min(x for x in (typed.find(b"\r"), typed.find(b"\n")) if x >= 0)
                line = bytes(typed[:i]).decode("latin1").strip()
                del typed[:i + 1]
            else:
                line = None
        if line is None:
            time.sleep(0.02)
            continue
        out = answer(line)
        conn.sendall(line.encode("latin1", "replace") + b"\r\n"
                     + ((out.encode() + b"\r\n") if out else b"") + PROMPT)
except (OSError, BrokenPipeError):
    pass
finally:
    stop.set()
    try:
        conn.close()
    except OSError:
        pass
    srv.close()
