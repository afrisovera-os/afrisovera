#!/usr/bin/env python3
"""GRADER 3.518 -- does the 3.517 absorption split DIVERGE?

3.517 landed `RX_RAW_COUNT` (10 sites, each immediately after the RX used-elem
dequeue, before `parse_tcp` and before the 4-tuple filter) beside the existing
matched `RX_COUNT` (10 sites, each the raw site's sibling within 30 lines
below, reached only if the frame passes the filters). 3.516 pre-registered the
condition the split has to meet: the two counters must be SHOWN to diverge, or
the split buys nothing and must be reverted.

3.517 could not read them: `sink-probe-3488.sh:103` filters the guest serial
through `grep -iE "sink|usage|LISTEN|SYN|ESTABLISHED|bytes|elapsed|rx frames"`
and `[net] rx raw N matched M` matches NONE of those tokens, so the line was
printed by the guest and dropped by the harness. Editing that probe to see it
is forbidden (2.765 rule 5) -- hence a NEW caller and THIS grader.

This module is PURE: it grades TEXT, never a live boot, so both directions are
reachable offline before a boot is spent (Law 9, the 2.766 lesson).

    0   RXSPLIT-DIVERGES     raw > matched -- frames come off the ring that the
                             matched counter never sees; the split is load-bearing
   10   RXSPLIT-IDENTICAL    raw == matched -- the split buys NOTHING on this run
                             and 3.517's prediction is FALSIFIED
    2   RXSPLIT-IMPOSSIBLE   raw < matched -- a tally taken BEFORE the filter
                             cannot trail one taken after it; the wiring is wrong
    1   RXSPLIT-UNOBSERVED   no `rx raw` line in the serial at all -- harness,
                             not a verdict about the counters

ANCHORED AT COLUMN 0 of the guest's own line, and the numbers are read off THAT
line only -- never off a banner that restates them (the 2.994 hazard). If the
serial carries several `rx raw` lines (the rig asks `net` once, but a future
caller may ask twice) the LAST one is graded and the count is reported, so a
reader can never mistake an early line for the final tally.
"""
import re
import sys

LINE = re.compile(r"^\[net\] rx raw (\d+) matched (\d+)\s*$", re.M)


def find_all(text):
    """Every (raw, matched) pair the serial carries, in order."""
    return [(int(a), int(b)) for a, b in LINE.findall(text)]


def grade(text):
    """(exit_code, label, raw, matched, n_lines) -- pure, no I/O."""
    hits = find_all(text)
    if not hits:
        return 1, "RXSPLIT-UNOBSERVED", None, None, 0
    raw, matched = hits[-1]
    if raw < matched:
        return 2, "RXSPLIT-IMPOSSIBLE", raw, matched, len(hits)
    if raw == matched:
        return 10, "RXSPLIT-IDENTICAL", raw, matched, len(hits)
    return 0, "RXSPLIT-DIVERGES", raw, matched, len(hits)


REAL_TAIL = """[net] virtio-net up: MAC 52:54:00:12:34:56, rx frames 45
[net] rx raw 61 matched 45
[net] 'net arp' | 'net ping' | 'net dns [name]'
"""

# The 3.517 artefact VERBATIM (.sink-3517.out): the line is absent because the
# probe's grep dropped it. The grader must call that UNOBSERVED, never a verdict.
ARTEFACT_3517 = """  [net] sink: 8704 byte(s) sunk in 17 segment(s) over 10339 ms
  [net] sink: rate 841 bytes/s
  [net] virtio-net up: MAC 52:54:00:12:34:56, rx frames 45
"""


def selftest():
    arms = [
        ("diverges-real-shape", REAL_TAIL, 0, 61, 45),
        ("identical-buys-nothing", "[net] rx raw 45 matched 45\n", 10, 45, 45),
        ("impossible-raw-trails", "[net] rx raw 44 matched 45\n", 2, 44, 45),
        ("absent-line", ARTEFACT_3517, 1, None, None),
        ("boundary-one-apart", "[net] rx raw 46 matched 45\n", 0, 46, 45),
        # The line must be ANCHORED: the same numbers inside a restating banner
        # must NOT be graded (the 2.994 hazard -- one derivation graded twice).
        ("indented-copy-not-graded", "  [net] rx raw 61 matched 45\n", 1, None, None),
        ("prefixed-copy-not-graded", "peer says [net] rx raw 61 matched 45\n", 1, None, None),
        # Trailing junk on the line is not the guest's line either.
        ("trailing-junk-not-graded", "[net] rx raw 61 matched 45 frames\n", 1, None, None),
        ("zero-zero-is-identical", "[net] rx raw 0 matched 0\n", 10, 0, 0),
    ]
    ok = fail = 0
    for name, text, want_rc, want_raw, want_matched in arms:
        rc, label, raw, matched, _n = grade(text)
        if (rc, raw, matched) == (want_rc, want_raw, want_matched):
            ok += 1
            print("  ok   %-26s -> %s (%s/%s)" % (name, label, raw, matched))
        else:
            fail += 1
            print("  FAIL %-26s -> rc=%s raw=%s matched=%s (wanted rc=%s %s/%s)"
                  % (name, rc, raw, matched, want_rc, want_raw, want_matched))

    # Multi-line arm: the LAST pair is the graded one and the count is reported.
    rc, label, raw, matched, n = grade(
        "[net] rx raw 10 matched 10\n[net] rx raw 61 matched 45\n")
    if (rc, raw, matched, n) == (0, 61, 45, 2):
        ok += 1
        print("  ok   %-26s -> %s (%s/%s over %d line(s))"
              % ("last-line-wins", label, raw, matched, n))
    else:
        fail += 1
        print("  FAIL last-line-wins -> rc=%s %s/%s n=%s" % (rc, raw, matched, n))

    # All four codes must be REACHED by the arms above, not merely defined.
    reached = {grade(t)[0] for _n, t, *_r in arms}
    for code in (0, 1, 2, 10):
        if code in reached:
            ok += 1
            print("  ok   code %-2d reached" % code)
        else:
            fail += 1
            print("  FAIL code %d never reached by any arm" % code)

    print("GRADER SELFTEST: %d ok / %d FAIL" % (ok, fail))
    return 0 if fail == 0 else 1


def main(argv):
    if len(argv) > 1 and argv[1] == "--selftest":
        return selftest()
    if len(argv) > 2 and argv[1] == "--grade":
        with open(argv[2], "r", encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        rc, label, raw, matched, n = grade(text)
        print("RXSPLIT: %s raw=%s matched=%s lines=%d"
              % (label, raw, matched, n))
        if raw is not None and matched is not None:
            print("RXSPLIT-DELTA: %d frame(s) came off the ring uncounted by the "
                  "matched tally" % (raw - matched))
        return rc
    print("usage: rxsplit-grade-3518.py --selftest | --grade <serial-file>")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
