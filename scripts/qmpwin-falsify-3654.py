#!/usr/bin/env python3
"""3.654 -- falsify-first on the census's WORST-EXPOSURE window.

    scripts/shell-demo.py  py-timeout-kw :320  10.00s  exposure 675 suites
    q = socket.create_connection(("127.0.0.1", qmp_port), timeout=10)

docs/audit-2.773-driver-windows.md rules this row "the QMP connect; the
readline() at :210 inherits it".  This asks the quantitative question that
wording leaves open: is 10 s a BOUND on the screenshot block, or only a
per-recv timeout that a dribbling peer never trips?

It runs the REAL scripts/shell-demo.py, UNEDITED (2.765 rule 5: if the thing
under test already exists in the tree, CALLING it is the only permitted form),
against a FAKE serial guest and four controlled QMP peers.  ZERO QEMU boots --
nothing here needs a kernel, because the quantity is a host-side socket window.

Arms: A control (healthy QMP) / B blackhole (accepts, never writes) /
      C refused (nothing listening) / D dribble (a byte every 2.5 s, no newline).
Every arm ALSO grades that the serial capture still reached stdout -- 2.772's
rule that the picture may fail but the capture may not.
"""
import re
import socket
import subprocess
import sys
import threading
import time
import os

HERE = os.path.dirname(os.path.abspath(__file__))
DEMO = os.path.join(HERE, "shell-demo.py")
BANNER = b"\r\n[fake guest] AfriSh ready\r\nafrisovera> "
MARK = "FALSIFY-CAPTURE-MARK"          # must survive to stdout in EVERY arm


def _listener():
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", 0))
    s.listen(4)
    return s, s.getsockname()[1]


def fake_serial(sock, stop):
    """A guest that prints a banner with the prompt, then answers each command."""
    sock.settimeout(30)
    try:
        c, _ = sock.accept()
    except OSError:
        return
    c.settimeout(0.3)
    c.sendall(BANNER)
    buf = b""
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
            # echo the command back (drain_until_prompt needs THIS command's
            # echo in the post-send bytes), then output, then the prompt.
            try:
                c.sendall(line + b"\r\n" + MARK.encode() + b" " + line + b"\r\nafrisovera> ")
            except OSError:
                return
    try:
        c.close()
    except OSError:
        pass


def qmp_peer(sock, mode, stop):
    sock.settimeout(30)
    try:
        c, _ = sock.accept()
    except OSError:
        return
    if mode == "control":
        try:
            c.sendall(b'{"QMP":{"version":{}}}\n')
            c.recv(4096)                              # qmp_capabilities
            c.sendall(b'{"return": {}}\n')
            c.recv(4096)                              # screendump
            c.sendall(b'{"return": {}}\n')
        except OSError:
            pass
    elif mode == "blackhole":
        # accept and go silent: the connect succeeds instantly, the FIRST
        # readline() is where the 10 s is actually spent.
        while not stop.wait(0.2):
            pass
    elif mode == "dribble":
        # a byte every 2.5 s and never a newline: each recv returns well inside
        # the 10 s socket timeout, so the timeout never expires.
        for _ in range(6):
            if stop.is_set():
                break
            try:
                c.sendall(b".")
            except OSError:
                break
            stop.wait(2.5)
    try:
        c.close()
    except OSError:
        pass


def run_arm(name, mode):
    stop = threading.Event()
    ser_s, ser_p = _listener()
    threads = [threading.Thread(target=fake_serial, args=(ser_s, stop), daemon=True)]
    if mode == "refused":
        # bind, learn the port, then CLOSE it so nothing is listening.
        tmp, qmp_p = _listener()
        tmp.close()
        qmp_s = None
    else:
        qmp_s, qmp_p = _listener()
        threads.append(threading.Thread(target=qmp_peer, args=(qmp_s, mode, stop), daemon=True))
    for t in threads:
        t.start()

    out_png = os.path.join(os.environ.get("TMPDIR", "/tmp"), "qmpwin-3654-%s.png" % name)
    t0 = time.time()
    p = subprocess.run([sys.executable, DEMO, str(ser_p), str(qmp_p), out_png, "about"],
                       capture_output=True, timeout=180)
    wall = time.time() - t0
    stop.set()
    for s in (ser_s, qmp_s):
        try:
            if s:
                s.close()
        except OSError:
            pass
    for t in threads:
        t.join(timeout=5)
    return {
        "arm": name, "wall": wall, "rc": p.returncode,
        "out": p.stdout.decode(errors="replace"),
        "err": p.stderr.decode(errors="replace"),
    }


def refused_connect_cost(samples=3):
    """What a REFUSED loopback connect actually costs on THIS host.

    3.654 PREDICTED "~0 s, ConnectionRefusedError is immediate" and was WRONG:
    measured 2.0435 / 2.0183 / 2.0426 / 2.0208 / 2.0302 s on the dev box -- a
    tight ~2.03 s, which is Windows retransmitting the loopback SYN before it
    returns WSAECONNREFUSED, not scheduler noise.  The number is re-derived
    here on every run rather than pinned, because it is a HOST property and a
    box that returns instantly must not fail this arm.
    """
    out = []
    for _ in range(samples):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        dead = s.getsockname()[1]
        s.close()
        t0 = time.time()
        try:
            socket.create_connection(("127.0.0.1", dead), timeout=10).close()
        except OSError:
            pass
        out.append(time.time() - t0)
    return min(out), max(out)


def main():
    print("3.654 QMP-window falsifier -- driving the REAL scripts/shell-demo.py, unedited")
    print("     zero QEMU boots; the quantity under test is a host-side socket window\n")
    arms = [("A-control", "control"), ("B-blackhole", "blackhole"),
            ("C-refused", "refused"), ("D-dribble", "dribble")]
    res = {}
    for name, mode in arms:
        r = run_arm(name, mode)
        res[name] = r
        skipped = [l for l in r["err"].splitlines() if "screendump skipped" in l]
        shot = [l for l in r["out"].splitlines() if l.startswith("screendump:")]
        print("  %-12s wall %6.2fs  rc=%d  capture=%s" % (
            name, r["wall"], r["rc"], "YES" if MARK in r["out"] else "**LOST**"))
        print("               %s" % (shot[0] if shot else (skipped[0].strip() if skipped
                                                          else "(no screendump line at all)")))
    print()

    ok, fail = 0, 0

    def grade(label, cond, detail):
        nonlocal ok, fail
        if cond:
            ok += 1
            print("  ok   %-34s %s" % (label, detail))
        else:
            fail += 1
            print("  FAIL %-34s %s" % (label, detail))

    wA = res["A-control"]["wall"]
    wB = res["B-blackhole"]["wall"]
    wC = res["C-refused"]["wall"]
    wD = res["D-dribble"]["wall"]

    grade("1 control takes the shot", any(l.startswith("screendump:")
          for l in res["A-control"]["out"].splitlines()),
          "wall %.2fs" % wA)
    grade("2 blackhole burns ~10s", 8.0 <= (wB - wA) <= 13.0,
          "B-A = %.2fs (the 10 s the census charges)" % (wB - wA))
    grade("3 blackhole names the skip", "screendump skipped" in res["B-blackhole"]["err"],
          "stderr carries the reason")
    rc_lo, rc_hi = refused_connect_cost()
    grade("4 refused != the 10s window",
          (wC - wA) < (rc_hi + 2.0) and (wC - wA) < 8.0,
          "C-A = %.2fs, bare refused connect %.2f-%.2fs on this host -- the "
          "CONNECT is not where the 10 s goes" % (wC - wA, rc_lo, rc_hi))
    grade("5 dribble EXCEEDS the 10s", (wD - wA) > (wB - wA) + 1.0,
          "D-A = %.2fs vs B-A = %.2fs" % (wD - wA, wB - wA))
    for name in res:
        grade("6 capture survives %s" % name.split("-")[0], MARK in res[name]["out"],
              "serial capture reached stdout (2.772's rule)")

    print("\n  %d ok / %d FAIL" % (ok, fail))
    if fail:
        print("QMPWIN-FALSIFY-3654: SOME ARMS FAILED")
        return 1
    print("QMPWIN-FALSIFY-3654-GREEN")
    return 0


if __name__ == "__main__":
    sys.exit(main())
