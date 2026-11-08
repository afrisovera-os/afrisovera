#!/usr/bin/env python3
"""3.657 -- falsify-first on 3.656's honest gap (a): the CONNECT sits OUTSIDE
the QMP block's deadline.

3.656 gave scripts/shell-demo.py's screenshot block ONE deadline, re-armed
inside the read loop, and its own SESSION_PLAN row recorded what it did NOT
close:

    (a) The connect's own 10 s sits OUTSIDE the deadline, which is armed after
        `create_connection` returns. Worst case is 10 s connect + 10 s block
        = 20 s.

This asks that question with a measurement instead of a reading, and it grades
the two rival claims AGAINST EACH OTHER so the tree names itself:

    arm 4  (PRE-repair claim)   the connect's cost ADDS to the block's window
    arm 5  (POST-repair claim)  the block is bounded by ONE budget, connect included

Exactly one of them can hold.  Before the repair 4 passes and 5 fails; after it,
5 passes and 4 fails.  Any other combination means the instrument, not the tree,
is wrong -- and it says so.

WHY NOT THE FILTERED ADDRESS THE 3.656 `=> NEXT:` CLAUSE ASKED FOR.  That clause
predicted "a peer that is unreachable-but-not-refusing ... has to cost ~20 s
BEFORE the fix and ~10 s after".  It is FALSE, and arm 7 re-derives why on every
run: a connect that hangs its whole window ENDS IN AN EXCEPTION, so the read
phase is never reached and the two windows are never spent in the same run.  A
filtered peer costs ~10 s before the repair and ~10 s after -- no differential
at all.  The 10 + 10 shape needs a connect that is SLOW **and then SUCCEEDS**,
which is what arm E builds.  (It also could not have been driven through
shell-demo.py, whose QMP host is the literal "127.0.0.1"; no knob was added to
production code to make this test possible.)

HOW ARM E MAKES A SLOW-BUT-SUCCESSFUL CONNECT, with no filter and no admin.
Measured on this host: a loopback connect to a port with no listener does NOT
fail instantly -- Windows retransmits the SYN for ~2.05 s before returning
WSAECONNREFUSED.  A listener that appears INSIDE that window catches a
retransmitted SYN and the connect succeeds, late (measured 0.53 / 1.04 / 1.53 s
for listeners armed at 0.5 / 1.0 / 1.5 s; armed at 2.5 s it refuses at 2.05 s).
So the arm has to place its listener within ~2 s of the driver's connect, and
that instant is derived rather than guessed: the fake guest DRIBBLES until a
chosen moment Y, and shell-demo.py's tail is `time.sleep(1.0); drain(); drain()`
with a 0.4 s socket timeout, so the two drains end 0.8 s after the last serial
byte and the connect is attempted at Y + 0.8.

Every arm ALSO grades that the serial capture still reached stdout -- 2.772's
rule that the picture may fail but the capture may not.

It drives the REAL scripts/shell-demo.py, UNEDITED (2.765 rule 5), and reuses
scripts/qmpwin-falsify-3654.py's peers by IMPORT rather than by copy, for the
same reason.  ZERO QEMU boots.

3.667 ADDS ARM F -- the sixth arm, for 3.659 gap (a)'s remaining half: the
`create_connection` call still passes the FULL `timeout=10` literal instead of
`max(0, deadline - time.time())` with `deadline` derived BEFORE the connect.
Arm E already proves the connect sits inside the one budget; arm F asks the
same question with the listener armed as LATE AS this harness reliably allows,
so the connect eats a much larger slice of the 10 s window and the "remaining
budget" question is sharper. Re-derived independently this tick (not trusted
from the docstring above): a bare two-thread probe (no subprocess, no serial
dribble, listener delay measured from a shared t0) caught the retransmit up to
a raw ~1.9-2.0 s delay and refused at 2.1 s, matching the ~2.05 s host ceiling
E's own docstring measured. But wired through THIS harness's own anchor
(`anchor_y + DRAIN_TAIL_S + late_s`, one more hop of process/bind latency than
the raw probe), the margin is tighter: 1.7 s and 1.75 s each armed valid 3/3,
while 1.8 s and 1.85 s each refused outright. `LATE_S2 = 1.7` is the value
actually used -- the harness's own measured ceiling, not the raw probe's.
Graded the same way as arm E (8: the connect validly
went late; 9: the block still respects ONE budget, not budget + late cost),
but NOT wired as a 4/5-style mutually-exclusive PRE/POST pair: unlike 3.657's
original gap, this tick's specific change (deadline-first, connect timeout
derived from it) cannot move arm F's wall clock, because the elapsed time
between deriving `deadline` and calling `create_connection` is a few
microseconds either way -- the "remaining budget" and "the full 10" are the
same number at the instant connect is called. Arm F's PRE vs POST wall times
are reported and compared in SESSION_PLAN.md; if they differ by more than
noise, that is itself a finding, not something to force.
"""
import importlib.util
import os
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DEMO = os.path.join(HERE, "shell-demo.py")

_spec = importlib.util.spec_from_file_location(
    "qmpwin3654", os.path.join(HERE, "qmpwin-falsify-3654.py"))
Q54 = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(Q54)
MARK = Q54.MARK
BANNER = Q54.BANNER
_listener = Q54._listener
qmp_peer = Q54.qmp_peer

HOLD_S = 3.0        # the guest dribbles this long past the last reply...
DRAIN_TAIL_S = 0.8  # ...and shell-demo's two 0.4 s drains end this much after it
LATE_S = 1.2        # listener armed this late => a connect that costs ~1.2 s
                    # and still succeeds (host ceiling measured at ~2.05 s)
LATE_S2 = 1.7       # 3.667 arm F: as late as THIS HARNESS reliably allows. A bare
                    # two-thread probe (no subprocess, no serial dribble) caught the
                    # retransmit up to a raw 1.9-2.0 s listener delay measured from a
                    # shared t0 -- but wired through this harness's own anchor
                    # (listener armed at anchor_y + DRAIN_TAIL_S + late_s, one more
                    # hop of process/bind latency than the raw probe), 1.7 s armed
                    # valid 3/3 and 1.75 s valid 3/3, while 1.8 s and 1.85 s refused
                    # 2/2 and 3/3 respectively. 1.7 s is the value actually used, not
                    # the raw probe's number -- pushes the connect's share of the
                    # 10 s budget from arm E's ~12% to ~17%.


def holding_serial(sock, stop, anchor):
    """fake_serial with an ANCHOR: it dribbles after the last reply, then stops.

    The dribble is CRLF only, so the capture the suites read is unchanged in
    content; what it buys is a known instant Y (recorded in `anchor`) from which
    shell-demo.py's `sleep(1.0); drain(); drain()` tail is deterministic.
    """
    sock.settimeout(30)
    try:
        c, _ = sock.accept()
    except OSError:
        return
    c.settimeout(0.3)
    c.sendall(BANNER)
    buf = b""
    replied = False
    while not stop.is_set():
        try:
            b = c.recv(4096)
            if not b:
                break
            buf += b
        except socket.timeout:
            continue
        except OSError:
            break
        while b"\r" in buf:
            line, buf = buf.split(b"\r", 1)
            try:
                c.sendall(line + b"\r\n" + MARK.encode() + b" " + line
                          + b"\r\nafrisovera> ")
            except OSError:
                return
            replied = True
        if replied:
            break
    if replied:
        end = time.time() + HOLD_S
        while time.time() < end and not stop.is_set():
            try:
                c.sendall(b"\r\n")
            except OSError:
                break
            stop.wait(0.25)
        anchor["y"] = time.time()          # last serial byte leaves HERE
        anchor["evt"].set()
    while not stop.wait(0.2):
        pass
    try:
        c.close()
    except OSError:
        pass


def late_listener(port, stop, anchor, mode="blackhole", late_s=LATE_S):
    """Bind+listen only AFTER the driver's connect is already in flight."""
    if not anchor["evt"].wait(60):
        return
    left = (anchor.get("y", time.time()) + DRAIN_TAIL_S + late_s) - time.time()
    if left > 0:
        stop.wait(left)
    if stop.is_set():
        return
    try:
        lst = socket.socket()
        lst.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        lst.bind(("127.0.0.1", port))
        lst.listen(4)
    except OSError as e:
        anchor["bind_err"] = repr(e)
        return
    anchor["armed"] = time.time()
    qmp_peer(lst, mode, stop)
    try:
        lst.close()
    except OSError:
        pass


def run_arm(name, mode):
    """mode: control | blackhole | slowconnect | slowconnect-late."""
    stop = threading.Event()
    anchor = {"evt": threading.Event()}
    ser_s, ser_p = _listener()
    threads = [threading.Thread(target=holding_serial,
                                args=(ser_s, stop, anchor), daemon=True)]
    qmp_s = None
    if mode in ("slowconnect", "slowconnect-late"):
        tmp, qmp_p = _listener()           # learn a free port, then release it
        tmp.close()
        late_s = LATE_S2 if mode == "slowconnect-late" else LATE_S
        threads.append(threading.Thread(target=late_listener,
                                        args=(qmp_p, stop, anchor),
                                        kwargs={"late_s": late_s}, daemon=True))
    else:
        qmp_s, qmp_p = _listener()
        threads.append(threading.Thread(target=qmp_peer,
                                        args=(qmp_s, mode, stop), daemon=True))
    for t in threads:
        t.start()

    out_png = os.path.join(os.environ.get("TMPDIR", "/tmp"),
                           "qmpconn-3657-%s.png" % name)
    t0 = time.time()
    p = subprocess.run([sys.executable, DEMO, str(ser_p), str(qmp_p), out_png, "about"],
                       capture_output=True, timeout=240)
    wall = time.time() - t0
    stop.set()
    anchor["evt"].set()
    for s in (ser_s, qmp_s):
        try:
            if s:
                s.close()
        except OSError:
            pass
    for t in threads:
        t.join(timeout=5)
    return {"arm": name, "wall": wall, "rc": p.returncode,
            "out": p.stdout.decode(errors="replace"),
            "err": p.stderr.decode(errors="replace"),
            "anchor": anchor}


def filtered_connect_shape(timeout=4.0):
    """Re-derive, every run, WHY a filtered peer cannot show the 10+10 shape.

    Returns (elapsed, outcome).  A filtered address burns the WHOLE window and
    then RAISES -- so the block's read phase is never entered and the connect
    window and the read window are never additive in one run.  192.0.2.1 is
    RFC 5737 TEST-NET-1: reserved for documentation, routed nowhere.
    """
    t0 = time.time()
    try:
        socket.create_connection(("192.0.2.1", 4444), timeout=timeout).close()
        return time.time() - t0, "CONNECTED"
    except Exception as e:
        return time.time() - t0, type(e).__name__


def skip_reason(r):
    for line in r["err"].splitlines():
        if "screendump skipped" in line:
            return line.strip()
    return ""


def main():
    print("3.657 QMP-CONNECT falsifier -- driving the REAL scripts/shell-demo.py, unedited")
    print("     the question: is the CONNECT inside the block's budget, or on top of it?\n")
    res = {}
    for name, mode in [("A-control", "control"), ("B-blackhole", "blackhole"),
                       ("E-slowconnect", "slowconnect"),
                       ("F-slowconnect-late", "slowconnect-late")]:
        r = run_arm(name, mode)
        res[name] = r
        shot = [l for l in r["out"].splitlines() if l.startswith("screendump:")]
        print("  %-14s wall %6.2fs  rc=%d  capture=%s" % (
            name, r["wall"], r["rc"], "YES" if MARK in r["out"] else "**LOST**"))
        print("                 %s" % (shot[0] if shot else
                                       (skip_reason(r) or "(no screendump line at all)")))
    print()

    ok, fail = 0, 0
    verdicts = {}

    def grade(n, label, cond, detail):
        nonlocal ok, fail
        verdicts[n] = verdicts.get(n, True) and bool(cond)
        if cond:
            ok += 1
            print("  ok   %d %-36s %s" % (n, label, detail))
        else:
            fail += 1
            print("  FAIL %d %-36s %s" % (n, label, detail))

    wA = res["A-control"]["wall"]
    wB = res["B-blackhole"]["wall"]
    wE = res["E-slowconnect"]["wall"]
    wF = res["F-slowconnect-late"]["wall"]
    budget = wB - wA                      # the block's window, MEASURED not typed
    eanch = res["E-slowconnect"]["anchor"]
    e_reason = skip_reason(res["E-slowconnect"])
    fanch = res["F-slowconnect-late"]["anchor"]
    f_reason = skip_reason(res["F-slowconnect-late"])

    grade(1, "control takes the shot",
          any(l.startswith("screendump:") for l in res["A-control"]["out"].splitlines()),
          "wall %.2fs" % wA)
    grade(2, "blackhole burns one whole budget", 8.0 <= budget <= 13.0,
          "B-A = %.2fs -- this is the budget the rest is graded against" % budget)
    grade(3, "E's connect SUCCEEDED (arm valid)",
          "ConnectionRefused" not in e_reason and e_reason != "" and "armed" in eanch,
          "listener armed %s; skip reason %r"
          % ("yes" if "armed" in eanch else "NO (%s)" % eanch.get("bind_err"),
             e_reason[-58:]))
    grade(4, "PRE-repair: connect ADDS to the window",
          (wE - wB) >= 0.6 * LATE_S,
          "E-B = %+.2fs vs the %.1fs late connect" % (wE - wB, LATE_S))
    grade(5, "POST-repair: ONE budget, connect included",
          abs(wE - wB) < 0.6 * LATE_S,
          "E-B = %+.2fs, E-A = %.2fs vs budget %.2fs" % (wE - wB, wE - wA, budget))
    for name in res:
        grade(6, "capture survives %s" % name.split("-")[0],
              MARK in res[name]["out"], "serial capture reached stdout (2.772's rule)")
    grade(8, "F's connect SUCCEEDED near the ceiling (arm valid)",
          "ConnectionRefused" not in f_reason and f_reason != "" and "armed" in fanch,
          "listener armed %s; skip reason %r"
          % ("yes" if "armed" in fanch else "NO (%s)" % fanch.get("bind_err"),
             f_reason[-58:]))
    grade(9, "F respects the ONE-budget bound (not budget + late cost)",
          abs(wF - wA - budget) < max(1.0, 0.6 * LATE_S2),
          "F-A = %.2fs vs budget %.2fs (late connect ~%.1fs would add %.2fs if OUTSIDE "
          "the budget)" % (wF - wA, budget, LATE_S2, wF - wA - budget))
    fel, fout = filtered_connect_shape()
    grade(7, "a FILTERED peer cannot show 10+10",
          fout != "CONNECTED" and fel >= 3.5,
          "192.0.2.1 burned %.2fs of a 4.00s window then raised %s -- the read "
          "phase is unreachable, so the windows never add" % (fel, fout))

    print("\n  %d ok / %d FAIL" % (ok, fail))
    if verdicts.get(4) and not verdicts.get(5):
        print("VERDICT: this tree spends CONNECT + BUDGET (the 3.656 gap (a) defect)")
        print("QMPCONN-FALSIFY-3657: PRE-REPAIR")
        return 10
    if verdicts.get(5) and not verdicts.get(4):
        if fail == 1:
            print("VERDICT: this tree spends ONE budget total, connect included")
            print("QMPCONN-FALSIFY-3657-REPAIRED")
            return 0
        print("VERDICT: repaired, but %d arms failed -- read them above" % fail)
        return 2
    print("VERDICT: INDETERMINATE -- arms 4 and 5 did not disagree; the instrument, "
          "not the tree, is what to fix")
    return 3


if __name__ == "__main__":
    sys.exit(main())
