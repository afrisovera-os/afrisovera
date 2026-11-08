#!/usr/bin/env python3
"""Phase 2.771 — a stand-in for `qemu-system-aarch64` that replays a REAL capture
at a chosen pace, so the `timeout N qemu ... ; grep` shape can be falsified with
ZERO boots.

WHY THIS EXISTS
---------------
The fix owed for `pi`/`dhcp`/`config` (audit-2.770 §7) is "replace the graded
window with a read that returns on the strings the suite needs, keeping
`timeout` as a ceiling only".  The kingdom rule is that a fix ships only after
the UNPATCHED shape has been SEEN to fail — and the 684-suite walk is live, so
this box cannot spare a QEMU boot (a contending guest would red a row for no
kernel reason, which is the exact defect under audit).

So the guest is replaced, not the harness.  This program:

  * accepts and IGNORES arbitrary qemu-style argv, so it drops into the suites'
    own command line unchanged;
  * replays the lines of a capture that a REAL boot produced
    (.rat-logs-2770/mansa-pi.log is 25 lines from this walk's own row-3 run) at
    REPLAY_PACE_MS per line;
  * then IDLES FOREVER, exactly as the raspi3b guest does — it never stops, it
    free-runs its scheduler.  That is what makes the constant a graded window
    rather than a ceiling, and it means both shapes must be exercised against a
    writer that outlives them.

It is a pacer, NOT a model of QEMU.  It proves nothing about the kernel; the
subject is the harness shape.  What it can be trusted for is exactly one claim:
"at this arrival rate, this many of the asserted lines are in the file when the
window closes" — which is arithmetic on bytes, not a simulation.

ENV (not flags — flags would collide with the qemu argv it must accept)
  REPLAY_LOG       path to the capture to replay   (required)
  REPLAY_PACE_MS   ms between lines                (default 0 = as fast as possible)
  REPLAY_DROP      regex; matching lines are NEVER printed.  This is the
                   genuinely-broken guest: it stands in for a kernel that really
                   did not arm its timer, and it is how the fix is checked for
                   the failure mode that would matter most — a ceiling that
                   swallows a real fault and reports itself instead of the
                   suite's own named assertion.
  REPLAY_HOLD_S    seconds to idle after the last line (default 3600)
"""
import os
import re
import sys
import time

log = os.environ.get("REPLAY_LOG")
if not log:
    sys.stderr.write("replay-guest: REPLAY_LOG is unset\n")
    sys.exit(2)

pace = float(os.environ.get("REPLAY_PACE_MS", "0")) / 1000.0
drop = os.environ.get("REPLAY_DROP") or None
hold = float(os.environ.get("REPLAY_HOLD_S", "3600"))
drop_re = re.compile(drop) if drop else None

with open(log, "r", encoding="utf-8", errors="replace") as fh:
    lines = fh.read().splitlines()

for line in lines:
    if drop_re is not None and drop_re.search(line):
        continue
    # Line-buffered + explicit flush: the reader on the other side is a `grep`
    # over a growing file, so a buffered writer would make the pace a lie.
    sys.stdout.write(line + "\n")
    sys.stdout.flush()
    if pace:
        time.sleep(pace)

# The guest does not exit.  Neither does this.
time.sleep(hold)
