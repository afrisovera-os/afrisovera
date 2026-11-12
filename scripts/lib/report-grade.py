#!/usr/bin/env python3
"""Grade one `report` boot for scripts/pc-report-test.sh (docs/design/field-report.md).

    report-grade.py <serial log> <receiver log> <reports dir> <card> <port> <closed port>
    report-grade.py --selftest

<card> is e1000 or rtl: the probe section and boot line that must name the card.
Exit 0 = green. Every failure is printed as "  FAIL: ...".
"""
import os
import re
import sys

SECTIONS = ["date", "fwinfo", "screeninfo", "cpuinfo", "acpiinfo", "mem", "uptime", "ifconfig",
            "devices", "lsvirtio", "lsusb", "lsps2", "lsnvme", "lsahci", "lse1000", "lsrtl", "lssd",
            "lshda", "wifi"]
CARD = {
    "e1000": ("lse1000", "82574L", "[e1000] 82574L"),
    "rtl": ("lsrtl", "RTL8139C+", "[rtl] RTL8139C+"),
}


def clean(t):
    return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", t.replace("\r", ""))


def grade(serial, recv, report, card, port, closed):
    """serial/recv: text; report: bytes or None. Returns a list of failures."""
    bad = []
    s = clean(serial)
    has = lambda pat: re.search(pat, s, re.M)
    if not has(r"^report: send this PC's details to your main PC"):
        bad.append("bare `report` did not explain itself")
    # QEMU's user network answers a closed host port with RST on some hosts and with silence
    # on others (Windows); either way the guest must give up in plain words
    if not has(r"^report: NOT SENT: 10\.0\.2\.2 (is there but nothing listens on port %d; start tools/report-receiver\.py first"
               r"|never answered; its firewall may block Python \(allow it on private networks\))$" % closed):
        bad.append("a report to a closed port was not refused in plain words")
    if not has(r"^\[report\] from 10\.0\.2\.15, receiver 10\.0\.2\.2 is at ([0-9a-f]{2}:){5}[0-9a-f]{2}$"):
        bad.append("the receiver's MAC was not found by ARP (on-link route)")
    m = has(r"^\[report\] SENT (\d+) bytes to 10\.0\.2\.2:%d in (\d+) segments \((\d+) resent\), \d+ ms$" % port)
    if not m:
        bad.append("the guest did not report the report SENT (with its close answered)")
    if has(r"KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)"):
        bad.append("the kernel faulted")
    rm = re.search(r"^Saved (\d+) bytes from 127\.0\.0\.1 to (.*?)(  \(INCOMPLETE\))?$", recv, re.M)
    if not rm:
        bad.append("the receiver saved nothing")
    elif rm.group(3):
        bad.append("the receiver called the report INCOMPLETE")
    if report is None:
        bad.append("no report file in the reports folder")
        return bad
    if m and rm and not (int(m.group(1)) == int(rm.group(1)) == len(report)):
        bad.append("sizes disagree: guest %s, receiver %s, file %d" % (m.group(1), rm.group(1), len(report)))
    if m and int(m.group(2)) < 8:
        bad.append("only %s segments: the windowed send was not exercised" % m.group(2))
    t = report.decode("utf-8", "replace")
    if not t.startswith("AFRISOVERA FIELD REPORT 1\n"):
        bad.append("the report does not start with its title line")
    if not re.search(r"^commit: ([0-9a-f]{8}(\+changes)?|unknown) \(", t, re.M):
        bad.append("no commit line")
    if not re.search(r"^build: (debug|release) x86_64 UEFI \(mansa-pc\)$", t, re.M):
        bad.append("no build line")
    if not re.search(r"^from: 10\.0\.2\.15 mac ([0-9a-f]{2}:){5}[0-9a-f]{2} \(%s\)$" % card, t, re.M):
        bad.append("no 'from' line naming the %s card" % card)
    cm = re.search(r"^cores: (\d+) \(ACPI\)   memory: (\d+) MiB$", t, re.M)
    if not cm or int(cm.group(1)) < 1 or int(cm.group(2)) < 16:
        bad.append("no plausible cores/memory line")
    pos = 0
    for name in SECTIONS + ["trouble lines in the console log"]:
        i = t.find("\n== %s ==\n" % name, pos)
        if i < 0:
            bad.append("section '%s' missing or out of order" % name)
        else:
            pos = i + 1
    sect, word, bootline = CARD[card]
    body = re.search(r"\n== %s ==\n(.*?)\n== " % sect, t, re.S)
    if not body or word not in body.group(1):
        bad.append("the %s probe did not name the %s" % (sect, word))
    bm = re.search(r"\n== boot log \(the first (\d+) bytes since power-on\) ==\n", t)
    if not bm or int(bm.group(1)) < 1000:
        bad.append("no boot log section")
    elif bootline not in t[bm.end():bm.end() + int(bm.group(1))]:
        bad.append("the boot log does not hold the card's own boot line")
    em = re.search(r"\n== end of report \((\d+) bytes before this line\) ==\n\Z", t)
    if not em:
        bad.append("the report does not end with its end line (cut short?)")
    elif int(em.group(1)) != len(report) - len(em.group(0).encode()):
        bad.append("the end line counts %s bytes but %d came before it: bytes lost or doubled"
                   % (em.group(1), len(report) - len(em.group(0).encode())))
    return bad


def load(serial, recv, rdir):
    rd = lambda p: open(p, encoding="utf-8", errors="replace").read() if os.path.exists(p) else ""
    files = sorted(os.listdir(rdir)) if os.path.isdir(rdir) else []
    report = open(os.path.join(rdir, files[-1]), "rb").read() if files else None
    return rd(serial), rd(recv), report


def fixture(card):
    sect, word, bootline = CARD[card]
    boot = ("[pc] AfriSovera boot\n%s (8139cp) at 00:02.0, MAC 52:54:00:12:34:56, link up\n" % bootline) * 40
    parts = ["AFRISOVERA FIELD REPORT 1\ncommit: 3a94a94c (2026-11-08T20:00:00+00:00)\n"
             "build: debug x86_64 UEFI (mansa-pc)\nfrom: 10.0.2.15 mac 52:54:00:12:34:56 (%s)\n"
             "to: 10.0.2.2:7700\ncores: 1 (ACPI)   memory: 255 MiB\n" % card]
    for n in SECTIONS:
        parts.append("\n== %s ==\n%s\n" % (n, word + " at 00:02.0" if n == sect else n + " output"))
    parts.append("\n== trouble lines in the console log ==\n(none)\n")
    parts.append("\n== boot log (the first %d bytes since power-on) ==\n%s" % (len(boot), boot))
    body = "".join(parts).encode()
    n = len(body)
    report = body + b"\n== end of report (%d bytes before this line) ==\n" % n
    serial = ("afrisovera> report\nreport: send this PC's details to your main PC over the home network.\n"
              "afrisovera> report 10.0.2.2:9\n"
              "report: NOT SENT: 10.0.2.2 is there but nothing listens on port 9; start tools/report-receiver.py first\n"
              "afrisovera> report 10.0.2.2:5000\n"
              "[report] from 10.0.2.15, receiver 10.0.2.2 is at 52:55:0a:00:02:02\n"
              "[report] connected to 10.0.2.2:5000, sending %d bytes\n"
              "[report] SENT %d bytes to 10.0.2.2:5000 in %d segments (0 resent), 120 ms\n"
              % (len(report), len(report), len(report) // 512 + 1))
    recv = "Saved %d bytes from 127.0.0.1 to /x/r.txt\n" % len(report)
    return serial, recv, report


def selftest():
    ok = bad = 0

    def arm(name, expect, serial, recv, report, card="e1000"):
        nonlocal ok, bad
        got = "green" if not grade(serial, recv, report, card, 5000, 9) else "red"
        if got == expect:
            ok += 1
            print("    arm %s: %s (expected %s) OK" % (name, got, expect))
        else:
            bad += 1
            print("    arm %s: %s but expected %s  <-- GRADER BUG" % (name, got, expect))
            for f in grade(serial, recv, report, card, 5000, 9):
                print("      " + f)

    s, r, rep = fixture("e1000")
    arm("green-e1000", "green", s, r, rep)
    s2, r2, rep2 = fixture("rtl")
    arm("green-rtl", "green", s2, r2, rep2, "rtl")
    arm("crlf-serial", "green", s.replace("\n", "\r\n"), r, rep)
    arm("wrong-card", "red", s, r, rep, "rtl")
    arm("no-usage", "red", s.replace("report: send this PC", "x"), r, rep)
    arm("not-refused", "red", s.replace("NOT SENT", "SENT"), r, rep)
    arm("closed-silent", "green", s.replace("is there but nothing listens on port 9; start tools/report-receiver.py first",
        "never answered; its firewall may block Python (allow it on private networks)"), r, rep)
    arm("no-arp", "red", s.replace("receiver 10.0.2.2 is at", "router 10.0.2.2 is at"), r, rep)
    arm("not-sent", "red", s.replace("[report] SENT", "[report] sent"), r, rep)
    arm("panic", "red", s + "[pc] PANIC: oops\n", r, rep)
    arm("receiver-nothing", "red", s, "", rep)
    arm("receiver-incomplete", "red", s, r.rstrip("\n") + "  (INCOMPLETE)\n", rep)
    arm("no-file", "red", s, r, None)
    arm("byte-lost", "red", s, r, rep[:300] + rep[301:])
    cut = rep[:len(rep) // 2]
    arm("cut-short", "red", s, r, cut)
    arm("byte-doubled", "red", s.replace(str(len(rep)), str(len(rep) + 1)),
        r.replace(str(len(rep)), str(len(rep) + 1)), rep[:200] + rep[199:])
    arm("section-missing", "red", s, r, rep.replace(b"\n== lsnvme ==\n", b"\n== xx ==\n"))
    arm("no-boot-line", "red", s, r, rep.replace(b"[e1000] 82574L", b"[e1000] 8257xL"))
    arm("few-segments", "red", s.replace(" in %d segments" % (len(rep) // 512 + 1), " in 3 segments"), r, rep)
    arm("empty", "red", "", "", None)
    print("  grader selftest: %d ok / %d FAIL" % (ok, bad))
    return bad == 0 and ok == 20


def main():
    if sys.argv[1:] == ["--selftest"]:
        return 0 if selftest() else 1
    serial, recv, rdir, card, port, closed = sys.argv[1:7]
    s, r, rep = load(serial, recv, rdir)
    fails = grade(s, r, rep, card, int(port), int(closed))
    for f in fails:
        print("  FAIL: " + f)
    if rep is not None and not fails:
        t = rep.decode("utf-8", "replace")
        print("  report: %d bytes; %s" % (len(rep), t.split("\n")[1]))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
