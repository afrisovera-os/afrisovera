#!/usr/bin/env python3
"""Census the FIXED-WINDOW class: suites that grade an assertion against a
constant number of seconds instead of against an event the guest emits.

WHY THIS FILE EXISTS. The class has been counted by hand three times in one
afternoon and every count came back LOW, in the flattering direction:

  * `scripts/lib/afrish-boot.sh:56` says the class has TWO members left.
  * 2.770b pass one anchored `^\\s*sleep\\s+N\\s*$` and reported FIVE — the
    anchor missed `shell-wfi-test.sh:16` because that line carries a trailing
    comment, and `shell-wfi` was the suite failing at that moment.
  * 2.770b's corrected pass reported EIGHT — it had learned the python
    read-window dialect (`drain(9)`) from a red, but could not see the
    `timeout N qemu-system-aarch64 … ; grep -q` dialect at all.

Three prose censuses, three under-counts. So the census is a SCRIPT now: the
next session runs THIS and reads the number, and when the number moves the
diff is in git rather than in someone's memory. (2.765 rule 5 in spirit: prose
names the hazard, only an artefact names the thing that avoids it.)

  python scripts/fixed-window-census.py            # report
  python scripts/fixed-window-census.py --check    # exit 1 if the class drifted

THE FOURTH UNDER-COUNT, AND WHY IT WAS NOT THE SCRIPT'S FAULT (2.773). Even as
a script the census stayed blind, because it opened `scripts/<name>-test.sh`
and nothing else. 2.772's red (`firstword`, a suite with NO window, immune by
every rule written since 2.758) was caused by a constant in the file the suites
SHARE: `shell-demo.py:200` connects to the QMP monitor with `timeout=10`, and
the `readline()` that inherits it is on line 210 while the capture is written
on line 212 — so a stalled screendump costs the suite its whole evidence and
reds it under the kernel's name. Measured exposure: 675 of 684 registered
suites invoke that driver, against ELEVEN for the entire per-suite class.

So this census now scans TWO populations and rules them separately:

  * the SUITE class (KNOWN, still eleven) — one suite, one window;
  * the DRIVER class (KNOWN_DRIVERS) — one constant, N suites, where N is
    measured from the suite files that reference the driver, never guessed.

They are kept apart on purpose: attributing a driver constant to each of its
675 users would drown the suite number that 2.771's owed conversions have to
move (11 → 8), and `--check` would stop being readable. Drift in EITHER
population exits 1.

TWO THINGS THE DRIVER PASS FORCED, both measured on the live tree first:
  1. `.py` files are read through `pystrip()`, which blanks comments and
     DOCSTRINGS — a docstring is PROSE, and the unstripped scan matched
     `sleep 8` inside shell-demo.py's own explanation of why `sleep 8` is
     wrong (line 134). Docstrings only, not every literal: see pystrip's own
     note for the twenty minutes this file spent hiding a live red behind an
     over-broad defence. Suites are `.sh` and are NOT stripped: their python
     lives in heredocs, which is where `py-drain` etc. are found.
  2. two dialects the shell-shaped ones could not express — `py-timeout-kw`
     (`timeout=N`, which finds the 2.772 constant) and `py-const-secs`
     (`NAME = float(os.environ.get(K, "90"))`, which finds four more). On the
     suite side that adds exactly one candidate, `tcp-retransmit:96`, ruled a
     CEILING and excluded WITH its reason. The ruled ELEVEN is unmoved, and
     that is checked rather than asserted: selftest arm 9 parses the member
     set out of the PREVIOUS COMMITTED census (via `git show`) and compares.

WHAT THE DRIVER PASS FOUND ON ITS FIRST RUN. Five graded windows in
shell-demo.py, every one of them at 675-suite exposure, against eleven
suite-local windows — and one of the five, `BOOT_QUIET = 3`, is the measured
cause of `rjust`'s red in the 2.770 walk (docs/audit-2.773-driver-windows.md).

THE FIFTH UNDER-COUNT: A WINDOW CAN BE SPELLED AS A SUBTRACTION (2.779). The
2.776 walk's row 681 (`tcp-serve2`) was red for a window this census scored at
`windows: 0` — `scripts/tcp-serve2-peer.py:516`, `time.time() - c.dialed >
retry_after`, whose constant is 1.5 s and lives three lines of control flow away
at `:413` in a TUPLE assignment (`retry_after, rto, syn_tries = 1.5, 1.5, 8`).
Every dialect above needs either a wait CALL on the line or an UPPERCASE-ish
NAME at the start of it; this window has neither, so the file holding the defect
was scored clean while a hand-written attribution blamed two innocent
`var-secs` budgets in the suite (docs/audit-2.777-tcp-serve2-red.md §"the
census's attribution was wrong, structurally"). The census counts SPELLINGS.

Two things that dialect forced, both of which are new machinery rather than one
more regex, and both measured on the live tree before they were written down:

  1. THE NUMBER IS NOT ON THE LINE. `py-elapsed-cmp`'s group(1) may be a NAME,
     so the scanner resolves names to numeric literals per file (bindings(),
     which reads tuple unpacking because that is how the 2.777 constant is
     spelled). A name that does not resolve is NEVER dropped: it is printed in
     the UNRESOLVED section, because silently skipping a candidate is the
     flattering direction this file exists to stop.
  2. THE THRESHOLD IS WRONG FOR THIS DIALECT. THRESHOLD = 3 s asks "can this
     constant swallow a BOOT". The 2.777 window is 1.5 s and swallowed a
     SYN/ACK by 218 ms; at the global threshold the new dialect would have
     found the defect and then filtered it out — a scanner that finds the
     thing and hides it is worse than one that never looked. So dialects may
     override the threshold (DIALECT_THRESHOLD), and selftest arm 20 asserts
     that the override is load-bearing rather than decorative.

WHAT COUNTS. A constant is in the class when the suite ASSERTS on output that
must have arrived before the constant expired. A constant is NOT in the class
when it is a CEILING — "up to N seconds, return as soon as the thing happens" —
because a ceiling that is hit is a real failure, not a timing artefact. Every
exclusion is listed in EXCLUDED below WITH ITS REASON and is printed on every
run: a census that silently drops a candidate is the defect this file exists
to stop.
"""

import argparse
import collections
import io
import os
import re
import sys
import tokenize

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
THRESHOLD = 3.0  # seconds; below this a constant cannot swallow a boot

# ---------------------------------------------------------------- dialects
# (label, regex whose group(1) is the constant, one-line description)
DIALECTS = [
    ("sh-sleep", re.compile(r"(?<![\w-])sleep\s+([0-9]+(?:\.[0-9]+)?)"),
     "shell `sleep N` — the graded window, or idle time bought on the HOST"),
    ("sh-read-t", re.compile(r"read\s+(?:-\w+\s+)*-t\s*([0-9]+(?:\.[0-9]+)?)"),
     "shell `read -t N` on a pipe/socket"),
    ("qemu-timeout", re.compile(r"(?<![\w-])timeout\s+(?:-\w+\s+)*([0-9]+(?:\.[0-9]+)?)"),
     "`timeout N qemu …` — QEMU is killed at N, then the log is grepped"),
    ("py-time-sleep", re.compile(r"time\.sleep\(\s*([0-9]+(?:\.[0-9]+)?)"),
     "python `time.sleep(N)` before a read"),
    ("py-drain", re.compile(r"\bdrain\(\s*([0-9]+(?:\.[0-9]+)?)"),
     "python `drain(N)` — read the serial socket for exactly N seconds"),
    ("py-deadline", re.compile(r"time\.time\(\)\s*\+\s*([0-9]+(?:\.[0-9]+)?)"),
     "python `time.time() + N` deadline used as a grading window -- BLIND to a "
     "variable-anchored spelling (`t_open = time.time(); deadline = t_open + N`); "
     "see the KNOWN_DRIVERS comment below for why that gap is decided not fixed"),
    ("py-settimeout", re.compile(r"settimeout\(\s*([0-9]+(?:\.[0-9]+)?)"),
     "python `socket.settimeout(N)`"),
    ("var-secs", re.compile(
        r"^\s*[A-Z_]*(?:SECS|SEC|WAIT|DELAY|WINDOW|SETTLE|BOOT|TIMEOUT)[A-Z_]*="
        r"\"?\$?\{?[^\"\n]*?([0-9]+(?:\.[0-9]+)?)"),
     "a named constant in seconds"),
    ("py-timeout-kw", re.compile(r"timeout\s*=\s*([0-9]+(?:\.[0-9]+)?)"),
     "python `timeout=N` keyword — socket.create_connection et al; it governs "
     "every later blocking call on that socket, not just the connect"),
    ("py-const-secs", re.compile(
        r"^\s*[A-Z_]*(?:SECS|SEC|WAIT|DELAY|WINDOW|SETTLE|BOOT|QUIET|CEILING"
        r"|IDLE|GRACE|TIMEOUT)[A-Z_]*\s*=\s*[^=]*?([0-9]+(?:\.[0-9]+)?)"),
     "a named constant in seconds, PYTHON side — `var-secs` cannot see these: "
     "it requires `NAME=` with no spaces (shell) and refuses to look inside "
     "quotes, and a python default lives in both (`os.environ.get(K, \"90\")`)"),
    ("py-elapsed-cmp", re.compile(
        r"time\.time\(\)\s*-\s*[A-Za-z_][\w.\[\]]*\s*(?:[<>]=?)\s*"
        r"([A-Za-z_]\w*|[0-9]+(?:\.[0-9]+)?)"),
     "python `time.time() - X > N` — a window spelled as a SUBTRACTION. "
     "group(1) is the RIGHT operand and may be a NAME, so it is resolved "
     "against the file's numeric bindings; unresolved names are printed, "
     "never dropped"),
]

# Per-dialect threshold overrides. THRESHOLD's question is "can this constant
# swallow a boot", which is the right question for `sleep`/`drain` and the
# WRONG one for a protocol rig's retry timer: 2.777's measured defect is 1.5 s
# and it cost a walk row by 218 ms. 1.0 s is chosen as the floor below which a
# comparison is a poll tick rather than a graded wait — `settimeout(1.0)` in the
# same file is exactly that. Arm 20 of the selftest asserts this line is what
# makes the 2.777 window visible, so deleting it fails the selftest rather than
# quietly shrinking the class.
DIALECT_THRESHOLD = {"py-elapsed-cmp": 1.0}

# ------------------------------------------------------------- exclusions
# suite -> (dialect, line, why this constant is a CEILING and not a window.
# Printed on every run. Never delete a line here to make the number smaller.)
EXCLUDED = {
    ("tcp-data", "py-settimeout"):
        "socket ceilings (90/30 s) — the peer returns the moment the bytes land",
    ("tcp-close", "py-settimeout"):
        "socket ceilings (90/30/25 s) — same shape as tcp-data",
    ("tcp-serve", "var-secs"):
        "GUEST_SECS/PEER_SECS are the serve BUDGET the suite hands the guest, "
        "not a window it grades output against",
    ("tcp-serve2", "var-secs"):
        "as tcp-serve, re-armed per connection",
    ("sleep", "sh-sleep"):
        "`sleep 300` is the GUEST's own command in milliseconds — the number is "
        "the thing under test, not a host wait",
    ("sleep-syscall", "sh-sleep"):
        "same: a guest-side sleep argument",
    ("tcp-retransmit", "py-timeout-kw"):
        "create_connection(...,timeout=20) at :96 is a CONNECT ceiling — it "
        "returns the instant QEMU accepts, and expiry prints 'the serial port "
        "never opened' and exits 1, which is a real failure, not a window. "
        "Found only when py-timeout-kw was added in 2.773; recorded so the "
        "twelfth candidate is never re-discovered as if it were new",
    ("pi", "qemu-timeout"):
        "`timeout 60` at :51 is a BACKSTOP around a backgrounded guest, not the "
        "quantity the suite grades against: 2.783 replaced `timeout 6 qemu; "
        "grep` with `qemu &; afrish_wait_markers; kill; one snapshot`, so the "
        "run returns the moment all six asserted strings are present (measured "
        "on the real raspi3b guest at 1079 / 1112 / 1253 / 2062 / 2078 / 2796 / "
        "3854 ms across seven boots — the first arrival times ever taken for "
        "this suite; the old shape burned 7135-9035 ms every time). `timeout` "
        "stays only in case `kill` cannot reap the child, and expiry of the "
        "WAIT returns 1 without exiting, so the suite's own assertion is what "
        "names a failure. KNOWN GAP, stated because this table's key is "
        "(suite, dialect): this entry would ALSO silence a genuinely new "
        "qemu-timeout window if one were ever added back to pi-test.sh. The "
        "invariant that would close it — a FIXED member must still call a wait "
        "seam — was the WRONG closer (a seam call says nothing about a SECOND window); the real one, EXCLUDED_COVERS, pins this excuse to its single 60 s constant and landed at 2.813",
    ("uart-rx-stress", "py-timeout-kw"):
        "`create_connection(..., timeout=10)` at :78 is the SERIAL connect "
        "INSIDE the retry loop 2.870 put around it — byte-for-byte the shape "
        "already ruled for shell-demo.py's connect_serial in EXCLUDED_DRIVERS. "
        "socket.timeout is an OSError, so expiry is CAUGHT and the connect is "
        "re-fired until BOOT_CEILING; the call returns the instant QEMU binds "
        "the port (measured: `serial socket accepted after 0.006s` on the new "
        "shape, against a `sleep 6` that the OLD shape rode to a "
        "ConnectionRefusedError under load). A ceiling reached is a NAMED "
        "failure with an exit 1, never a silent pass. CONDITIONAL on the "
        "connect-retry seam — delete the loop and this 10 s is a window again",
    ("uart-rx-stress", "py-elapsed-cmp"):
        "`time.time() - t_boot >= BOOT_CEILING` at :80 is the BACKSTOP on that "
        "same retry loop, not a quantity the suite grades against: the loop "
        "exits the moment the connect succeeds, and expiry prints `UART-RX-"
        "STRESS TEST FAILED (nothing listening on <port> after 60s: ...)` and "
        "exits 1. Ruled at 2.871, one dispatch after the conversion that "
        "created it — 2.870 left both constants COUNTED and said so, which is "
        "why the class read 73 s for this row. The per-trial `end = time.time() "
        "+ 3.0` at :177 was left COUNTED here on purpose (it was UNMEASURED at "
        "the time) — 3.701 measured it and it is ruled separately below, under "
        "its own dialect (py-deadline). CONDITIONAL on the connect-retry seam",
    ("uart-rx-stress", "py-deadline"):
        "`end = time.time() + 3.0` at :177 is a per-trial CEILING inside the "
        "300-trial loop, not a quantity the suite grades against: the `while "
        "time.time() < end:` loop (:178-186) breaks the instant the echoed "
        "reply ends in `afrisovera>` (:185-186), and only a trial that never "
        "gets that reply rides the full 3.0 s — which the suite then treats as "
        "a real loss (three more probes, then FAILED/exit 1 if the console "
        "stays deaf), never a silent pass. MEASURED at 3.701: a standalone probe "
        "(scripts/uart-trial-window-probe-3701.sh, unregistered, separate QEMU "
        "port 4886) replayed the identical 300-trial shape byte-for-byte "
        "(seed, CMD, chunking/delay, the same early-break condition) but "
        "printed each trial's elapsed time instead of discarding it: "
        "`300 of 300 answered, none rode the 3.0s ceiling`, "
        "min=0.0517s median=0.3043s p99=0.4382s max=0.4703s, a 2.5297 s margin "
        "(84% of the 3.0 s ceiling unused). One measured boot, not a proof for "
        "every host-load condition (2.766's contention precedent still applies), "
        "but it directly answers what 2.871 left UNMEASURED. CONDITIONAL on the "
        "per-trial early-break seam — delete the `break` and this excuse should "
        "stop applying",
    ("smpinfo", "qemu-timeout"):
        "`timeout 60` at Part B is a BACKSTOP around a backgrounded guest, not "
        "the quantity the suite grades against: 2.869 replaced `sleep 12; kill; "
        "grep` with `timeout 60 qemu &; afrish_wait_markers; kill; ONE "
        "snapshot`, the fourth suite to take the `pi` shape (2.783) after `dhcp` "
        "and `config` (2.788). SMP_RE is the grader's Part-B assertion list "
        "VERBATIM (`SMP: 2 cores online`, `secondary core Aff0 = 1`), so the "
        "wait cannot be satisfied by a different bar than the verdict. Measured "
        "back to back on the same kernel under a live 684-walk: the old shape "
        "took 12.094 s and the new one 0.540 s, both arms grepping 1 and 1 for "
        "the two markers, with the waiter printing `all 2 markers seen after "
        "405 ms` — per 2.783 that arrival number includes the waiter's OWN fork "
        "cost and is an UPPER BOUND on the guest, so the wall times are the "
        "honest quantity. `timeout` stays only in case `kill` cannot reap the "
        "child, and expiry of the WAIT returns 1 without exiting, so the "
        "grader's own `miss_b` names any failure. Same KNOWN GAP as pi's, "
        "dhcp's and config's entries, and bounded the same way: EXCLUDED_COVERS "
        "pins this excuse to its single 60 s constant and SEAM_REQUIRED holds "
        "it to still calling the seam",
    ("dhcp", "qemu-timeout"):
        "`timeout 60` at :38 is a BACKSTOP around a backgrounded guest, not the "
        "quantity the suite grades against: 2.788 replaced `timeout 8 qemu; "
        "grep` with `qemu &; afrish_wait_markers; kill; ONE snapshot`, the same "
        "shape `pi` took in 2.783, and the wait list IS the assertion list "
        "(DISCOVER sent / OFFER 10.0.2.15 / IP 10.0.2.15) so the wait cannot be "
        "satisfied by a different bar than the verdict. The old shape had ZERO "
        "measured margin — three runs of it took 8405 / 8417 / 8597 ms wall, "
        "every one riding `timeout 8` to its boundary, because it was really "
        "waiting for the whole boot demo to reach the shell prompt; the lease "
        "itself lands ~0.4 s in (markers at 406 / 418 ms across two boots, "
        "whole suite 1062 / 1333 ms wall). `timeout` stays only in case `kill` "
        "cannot reap the child, and expiry of the WAIT returns 1 without "
        "exiting, so the suite's own assertion names any failure. Same KNOWN "
        "GAP as pi's entry: this table's key is (suite, dialect), so this line "
        "would also silence a genuinely NEW qemu-timeout window added to "
        "dhcp-test.sh — the owed invariant (a FIXED member must still call a "
        "wait seam) was the wrong closer; EXCLUDED_COVERS closed it at 2.813",
    ("config", "qemu-timeout"):
        "`timeout 60` at :49 is a BACKSTOP around a backgrounded guest, not the "
        "quantity the suite grades against: 2.788 replaced `timeout 7 qemu; "
        "grep`, TWICE, with `qemu &; afrish_wait_markers; kill; ONE snapshot` "
        "per boot — the third suite to take the `pi` shape (2.783) after `dhcp`. "
        "Both wait lists ARE the assertion lists (CFG1_RE = `initialized; "
        "hostname=afrisovera boots=1`; CFG2_RE = `loaded from disk` + "
        "`boots=2`), so neither boot's wait can be satisfied by a different bar "
        "than its verdict. `ip=10.0.2.15` is deliberately in NEITHER list: it is "
        "printed as a bonus and graded `|| true`, and a wait list may only "
        "contain what the suite actually fails on. Measured: the old two-boot "
        "shape took 16077 ms wall (rc=0, one run off a `git stash` of this very "
        "patch); the new one took 9283 and 4672 ms, with the markers landing at "
        "3633 ms (boot 1, blank disk) and 2643 ms (boot 2, restore). Per 2.783, "
        "those printed arrival times include the waiter's OWN fork cost and are "
        "an upper bound on the guest, not a measurement of it — the wall times "
        "are the honest quantity. `timeout` stays only in case `kill` cannot "
        "reap the child, and expiry of the WAIT returns 1 without exiting, so "
        "the suite's own assertion names any failure. Same KNOWN GAP as pi's "
        "and dhcp's entries: this table's key is (suite, dialect), so this line "
        "would also silence a genuinely NEW qemu-timeout window added to "
        "config-test.sh. That gap is now owed by THREE entries, which is the "
        "argument for implementing the invariant (a FIXED member must still "
        "call a wait seam) rather than restating it a fourth time. DONE at 2.813, though not by that invariant: EXCLUDED_COVERS pins each excuse to the constants it was written for",
    ("persist", "var-secs"):
        "`AFRISH_BOOT_GRACE=10` at :26 is a CONDITIONAL HOLD, not a window: the "
        "suite never spends it on the path it grades. The constant is handed to "
        "scripts/shell-demo.py, where `grace` is reachable from exactly ONE "
        "branch (wait_ready, `not log and silent_for >= quiet` — ZERO bytes "
        "since attach); the happy path returns the instant PROMPT lands and "
        "never reads it, and any byte RETRACTS the suspicion before a keystroke. "
        "MEASURED (3.129, from already-recorded artefacts, no new load): across "
        "24 `.rat-logs-*` capture dirs these two suites left 32 logs and entered "
        "the branch in 0 of them — 0 `typing blind`, 0 retractions — while the "
        "SAME three greps over ALL captures in the SAME dirs return 23 / 11 / 8, "
        "in ten other suites. The grep demonstrably works; it never fires here. "
        "HONEST GAP, and the reason this excuse is conditional rather than a "
        "deletion: 0-of-32 is NEVER OBSERVED, not UNREACHABLE — a guest that "
        "booted before we attached would still spend the hold, and the 23 "
        "positive-control hits prove that case is real for other suites. So the "
        "seam is the RETRACTION (SEAM_REQUIRED `grace-retraction`), which spans "
        "two files: drop the hand-off here or the retraction in shell-demo.py "
        "and the hold becomes unconditional and this line stops silencing it. "
        "EXCLUDED_COVERS pins the excuse to its single 10 s constant",
    ("persist-mtime", "var-secs"):
        "`AFRISH_BOOT_GRACE=10` at :27 is the SAME constant as `persist`'s, "
        "handed to the same driver at the same seam, and it is ruled for the "
        "same measured reason: 0 of 32 recorded runs entered the branch that "
        "reads it, against a positive control of 23 in the same capture dirs. "
        "See the `persist` entry above for the measurement and its honest gap; "
        "this is a second suite, not a second argument. Same conditional seam "
        "(`grace-retraction`) and its own EXCLUDED_COVERS pin at 10 s",
    ("tcp-retransmit", "py-elapsed-cmp"):
        "`while time.time() - t0 < 120` at :102 is the SAME suite's prompt "
        "wait and the same shape as its exclusion above: the loop exits the "
        "instant `afrisovera>` lands (sys.exit(0) at :113), and expiry prints "
        "'NO SHELL PROMPT within 120s -- the guest never got far enough to be "
        "tested' and exits 1 — a real failure, not a timing artefact. It is "
        "the ONLY comparison-shaped candidate in the whole suite population "
        "(2.779), so the dialect's suite-side yield is one candidate and one "
        "ceiling; the class it actually moves is the driver class",
}

# ------------------------------------------------- what each excuse COVERS
# THE SECOND GAP THREE ENTRIES OWED, NOW PAID (2.813).
#
# EXCLUDED's key is (suite, dialect) — NOT (suite, dialect, line), whatever the
# comment above it says. So an excuse written for ONE constant silences EVERY
# constant of that dialect in that suite, and `pi`/`dhcp`/`config` each say so
# in prose: "this line would also silence a genuinely NEW qemu-timeout window
# added to <suite>-test.sh". All three name the closer as "a FIXED member must
# still call a wait seam" — but SEAM_REQUIRED does NOT close this. A suite can
# call `afrish_wait_markers` and ALSO grow a fresh `timeout 8 qemu`; the seam
# call says nothing about a second window. What closes it is pinning WHAT each
# excuse is allowed to cover, and this table is that pin.
#
# Values are a MULTISET, drawn down one hit at a time in scan order. A hit whose
# value is not in the pool — or a duplicate past the pinned count — is NOT
# excused: it falls through to the FIXED class and is announced by name. That
# closes value drift (a new `timeout 8` beside the ruled `timeout 60`) and count
# drift (a second `timeout 60`) with one mechanism, for all eleven keys rather
# than only the three that complained.
#
# Derived from a real scan of the tree, never hand-typed. `sleep-syscall` pins
# the EMPTY multiset because its excuse currently matches ZERO hits — a dead
# excuse, surfaced by this table rather than left to look load-bearing.
EXCLUDED_COVERS = {
    ('config', 'qemu-timeout'): (60.0,),
    ('dhcp', 'qemu-timeout'): (60.0,),
    ('persist', 'var-secs'): (10.0,),
    ('persist-mtime', 'var-secs'): (10.0,),
    ('pi', 'qemu-timeout'): (60.0,),
    ('smpinfo', 'qemu-timeout'): (60.0,),
    ('sleep', 'sh-sleep'): (300.0, 300.0),
    ('sleep-syscall', 'sh-sleep'): (),
    ('tcp-close', 'py-settimeout'): (25.0, 30.0, 90.0),
    ('tcp-data', 'py-settimeout'): (30.0, 90.0),
    ('tcp-retransmit', 'py-elapsed-cmp'): (120.0,),
    ('tcp-retransmit', 'py-timeout-kw'): (20.0,),
    ('tcp-serve', 'var-secs'): (30.0, 90.0),
    ('tcp-serve2', 'var-secs'): (25.0, 150.0),
    ('uart-rx-stress', 'py-deadline'): (3.0,),
    ('uart-rx-stress', 'py-elapsed-cmp'): (60.0,),
    ('uart-rx-stress', 'py-timeout-kw'): (10.0,),
}

# ------------------------------------------------------ the SEAM INVARIANT
# THE GAP THREE ENTRIES OWED, NOW PAID (2.789).
#
# `pi`, `dhcp` and `config` are excused above for ONE reason and it is not a
# property of the constant: the suite no longer GRADES against it, because it
# waits on the guest's own markers first and the `timeout` is only a backstop
# in case `kill` cannot reap. That reason is a CLAIM ABOUT THE SUITE'S TEXT,
# but this table's key is only (suite, dialect) — so the excuse would keep
# applying after the seam call was deleted, and a genuinely new fixed window
# in one of those three files would be silenced by a sentence describing a
# repair that is no longer there. Each of 2.783 / 2.788c / 2.788d wrote that
# gap down and left it; restating it a fourth time is not paying it.
#
# So the exclusion becomes CONDITIONAL: it holds only while the suite still
# CALLS the seam. Drop the call and the hit is not silenced — the suite walks
# straight back into the class and --check exits 1. The condition is the weak,
# honest one (the call is present in the file), NOT that the wait list is
# well-chosen; a census cannot grade a wait list, and pretending otherwise
# would put a second unfalsifiable claim where the first one was.
SEAM_CALL = re.compile(r"\bafrish_wait_markers?\b")

# 2.871: A SECOND SEAM SHAPE. `uart-rx-stress` is excused for the same KIND of
# reason as the four above — the suite no longer waits a fixed guess, it
# returns the moment the guest is there — but its repair is NOT a call to
# `afrish_wait_markers`; it is a connect RETRY LOOP bounded by BOOT_CEILING
# (shell-demo.py's connect_serial shape, already ruled in EXCLUDED_DRIVERS).
# Excusing it under the marker seam would have been a lie in the condition
# itself: delete the retry loop and `afrish_wait_markers` was never there to
# go missing, so the invariant would have vouched for a repair it never saw.
# So a seam is now NAMED per exclusion. Each entry lists the regexes that must
# ALL appear on a non-comment line; the condition stays the weak, honest one
# (the repair's text is still in the file), never a claim about its quality.
SEAMS = {
    "wait-markers": (
        "the suite calls afrish_wait_marker(s)",
        (SEAM_CALL,),
    ),
    "connect-retry": (
        "the connect still retries until BOOT_CEILING",
        (re.compile(r"while\s+ser\s+is\s+None\s*:"),
         re.compile(r"except\s+OSError"),
         re.compile(r"\bBOOT_CEILING\b")),
    ),
    # 3.701: A FOURTH SEAM SHAPE, and the first inside the per-trial loop rather
    # than the boot-connect one. `uart-rx-stress`'s per-trial `end = time.time()
    # + 3.0` (:177) is excused because the loop that spends it (:178-186) breaks
    # the instant the echoed reply ends in the prompt — delete that break and a
    # trial that answers instantly would still burn the full 3.0 s, so the
    # condition is the weak, honest one (the early-break text is still in the
    # file), same shape as connect-retry above, never a claim that 3.0 s is the
    # right ceiling.
    "echo-prompt-break": (
        "the per-trial loop still breaks early on the echoed prompt",
        (re.compile(r'bytes\(log\[start:\]\)\.rstrip\(\)\.endswith\(b"afrisovera>"\)'),
         re.compile(r"^\s*break\s*$")),
    ),
    # 3.130: A THIRD SEAM SHAPE, and the first one that does NOT live in the
    # suite. `persist` and `persist-mtime` do not spend their 10 s: the constant
    # is handed to shell-demo.py as AFRISH_BOOT_GRACE, and the hold it buys is
    # reachable from exactly ONE branch there (`not log and silent_for >= quiet`
    # — zero bytes since attach). Any byte RETRACTS it; the happy path returns
    # the instant PROMPT lands and never reads `grace` at all. 3.129 measured it:
    # over 24 capture dirs the two suites left 32 logs and entered the branch in
    # 0 of them, while the SAME grep over all captures in the SAME dirs returned
    # 23, in ten other suites — the grep works, it never fires on these two.
    #
    # So the excuse is a claim about TWO files, and the condition must be too.
    # The suite half is that it still hands the constant to that driver; the
    # driver half is that the retraction is still there. Vouching for only the
    # suite half would be the 2.871 lie again in a new place: delete the
    # retraction from shell-demo.py and a 10 s hold becomes unconditional, with
    # nothing in persist-test.sh changed to notice it.
    #
    # 0-of-32 is "never observed", NOT "unreachable" — a guest that booted
    # BEFORE we attached would still spend it, and the 23 positive-control hits
    # prove that case is real for OTHER suites. That is exactly why the excuse
    # is conditional on the RETRACTION and not on the branch being dead.
    "grace-retraction": (
        "the suite still hands BOOT_GRACE to shell-demo.py, and shell-demo.py "
        "still retracts the quiet-window guess on the first byte",
        (re.compile(r"\bAFRISH_BOOT_GRACE\b"),
         re.compile(r"shell-demo\.py")),
        {"scripts/shell-demo.py": (
            # the entry branch: the hold is reachable only from total silence
            re.compile(r"if\s+not\s+log\s+and\s+silent_for\s*>=\s*quiet"),
            # the retraction: any byte undoes the guess before a keystroke
            re.compile(r"if\s+t_suspect\s+is\s+not\s+None\s*:"),
            # the exit: the hold is bounded by `grace`, not by the ceiling alone
            re.compile(r"time\.time\(\)\s*-\s*t_suspect\s*>=\s*grace"),
        )},
    ),
}

# (suite, dialect) -> the seam its exclusion describes.
SEAM_REQUIRED = {
    ("pi", "qemu-timeout"): "wait-markers",
    ("dhcp", "qemu-timeout"): "wait-markers",
    ("config", "qemu-timeout"): "wait-markers",
    ("smpinfo", "qemu-timeout"): "wait-markers",
    ("uart-rx-stress", "py-timeout-kw"): "connect-retry",
    ("uart-rx-stress", "py-elapsed-cmp"): "connect-retry",
    ("uart-rx-stress", "py-deadline"): "echo-prompt-break",
    ("persist", "var-secs"): "grace-retraction",
    ("persist-mtime", "var-secs"): "grace-retraction",
}


def _live_lines(text):
    """Non-comment lines. See seam_present for why this is comment-blind."""
    return [ln for ln in text.splitlines() if not ln.lstrip().startswith("#")]


def seam_present(text, seam="wait-markers"):
    """True if this suite's source still CONTAINS the named repair.

    Comment-blind on purpose: a suite that only mentions the seam in prose
    would pass, and that is the failure this must not have. Lines whose first
    non-blank character is `#` are dropped before the search, so the three
    reasons in EXCLUDED above — which all name `afrish_wait_markers` — cannot
    vouch for a file by being quoted inside it.
    """
    live = _live_lines(text)
    if not all(any(rx.search(ln) for ln in live) for rx in SEAMS[seam][1]):
        return False
    # 3.130: a seam may also name OTHER files (the driver the suite hands its
    # constant to). Read from ROOT at call time, never from a cached copy, so
    # the condition is about the tree as it stands. A named file that is GONE
    # is a lost seam, not a pass.
    for rel, rxs in (SEAMS[seam][2] if len(SEAMS[seam]) > 2 else {}).items():
        path = os.path.join(ROOT, rel)
        if not os.path.exists(path):
            return False
        with io.open(path, encoding="utf-8", errors="replace") as fh:
            dlive = _live_lines(fh.read())
        if not all(any(rx.search(ln) for ln in dlive) for rx in rxs):
            return False
    return True


# ------------------------------------------------- the SHARED-DRIVER class
# Same idea, other population: a constant inside a file that MANY suites run.
# Keyed (driver-relpath, dialect, source-line-text) — the text, not the line
# number, because a patch that shifts lines must not read as drift, while the
# two `timeout=10` sites in shell-demo.py must stay distinguishable (they are
# the same dialect at the same value and only one of them is a member).
# NOTE ON THE KEY TEXT: `.py` sources are read through pystrip(), so string
# literals are blanked before a line is ever reported — the key below is the
# STRIPPED line, whitespace-collapsed by key_of(). That is deliberate: it makes
# the key survive both a re-indentation (2.772's patch re-nests exactly these
# statements) and any change to the literals inside the call, while still
# telling the two `timeout=10` sites apart, which line numbers alone would not
# do after a patch shifts them.
EXCLUDED_DRIVERS = {
    ("scripts/shell-demo.py", "py-timeout-kw",
     'return socket.create_connection(("127.0.0.1", port), timeout=10)'):
        "the SERIAL connect, inside connect_serial()'s retry loop: socket."
        "timeout is an OSError, so expiry is caught and retried until "
        "BOOT_CEILING. Returns the moment QEMU listens ⇒ ceiling, not window. "
        "The census corroborates this independently — see the guard= column",
    ("scripts/reorder-probe-2766.sh", "var-secs", "GUEST_SECS=30"):
        "serve BUDGET handed to the guest, as tcp-serve's own exclusion",
    ("scripts/reorder-probe-2766.sh", "var-secs", "PEER_SECS=90"):
        "peer-side budget, same shape",
    ("scripts/peerip-probe-2769.sh", "var-secs", "GUEST_SECS=25"):
        "serve budget handed to the guest, as above",
    ("scripts/peerip-probe-2769.sh", "var-secs", "PEER_SECS=75"):
        "peer-side budget, same shape",
    # 3.643 — suite 689 `rxsplit` arrived at 3.538 and brought the same three
    # budgets the two entries above already rule, in the same two positions.
    # Read at the call sites rather than by name: GUEST_SECS is argument 2 of
    # `net tcp sink $GPORT $GUEST_SECS` (:97), and PEER_SECS / PUSH_SECS are
    # arguments 3 and 4 of `sink-peer-3488.py "$PORTFILE" "$GPORT" "$PEER_SECS"
    # "$PUSH_SECS"` (:76). None is a quantity the probe grades against: the
    # verdict is `rxsplit-grade-3518.py --grade "$SERFILE"`, which reads the
    # guest's own census lines out of the serial.
    ("scripts/rxsplit-probe-3518.sh", "var-secs", "GUEST_SECS=20"):
        "sink BUDGET handed to the guest as arg 2 of `net tcp sink`, byte-for-"
        "byte the shape ruled for reorder-probe-2766.sh and peerip-probe-2769.sh",
    ("scripts/rxsplit-probe-3518.sh", "var-secs", "PEER_SECS=75"):
        "peer-side budget, same shape — sink-peer-3488.py:539 turns it into "
        "`deadline = time.time() + limit` and the loop ALSO breaks early on "
        "GIVEUP (:595), so it is the peer process's backstop, not a wait",
    ("scripts/rxsplit-probe-3518.sh", "var-secs", "PUSH_SECS=12"):
        "the peer's own push budget (sink-peer-3488.py:572 closes when it is "
        "spent and SAYS so: `peer: push window over`), i.e. how long the peer "
        "generates traffic — the probe grades the guest's census lines, never "
        "this number",
    # 3.669 — 3.668 registered suite `tcp-inuse`, which calls this probe
    # (tcp-inuse-test.sh:75, `PROBE="scripts/inuse-boot-probe-2859.sh"`) and
    # newly promoted it into the driver class. Check #6's own connect is a
    # liveness poll on 127.0.0.1, same shape as EXCLUDED_DRIVERS' shell-demo.py
    # entry above: it returns the instant the accept() succeeds, and a refused/
    # unreachable loopback connect fails with an immediate RST rather than
    # riding the 3s out. The failure branch is a NAMED result ("bad 6 foreign
    # holder no longer serves"), never silent — ceiling, not a graded window.
    ("scripts/inuse-boot-probe-2859.sh", "py-settimeout",
     "s=socket.socket(); s.settimeout(3)"):
        "check #6's liveness probe of the held port after the suite runs — "
        "returns on connect or on an immediate loopback refusal, and either "
        "way check #6 names the result ('ok'/'bad 6') rather than staying "
        "silent on expiry",
}

# 2.776 — FILE-level exclusions, and the granularity moved for a reason.
#
# 2.775 found that drivers() counts a MENTION as a RUN: its falsifier entered
# the driver class because reboot-test.sh's doc-comment cites it by path, and it
# ruled the two offending lines into EXCLUDED_DRIVERS by their TEXT. That works
# and it is a landmine — the key is a SENTENCE, so re-wording the sentence
# re-drifts the census and the repair is to edit the table rather than to learn
# anything. Same shape as the three selftest arms 2.775 had to repair, and the
# law it wrote: assert an INVARIANT, not a ruled quantity.
#
# The invariant is about the FILE. No entry in SUITES runs these; each is an
# offline falsifier or this census itself, visible only through a provenance
# line in a suite comment — a line the suite should keep, which is why this is
# ruled rather than reworded (2.775's reasoning, unchanged). drivers() is still
# NOT tightened to require an invocation shape: a rule that misses one genuine
# driver understates exposure, the flattering direction this class keeps failing
# in. Entries are named ONE AT A TIME — never a glob — and printed every run.
EXCLUDED_DRIVER_FILES = {
    "scripts/reboot-window-falsify-2775.sh":
        "2.775's offline falsifier for reboot-test.sh's drain(9); cited by a "
        "doc-comment in that suite. Migrated here from two EXCLUDED_DRIVERS "
        "keys that were the falsifier's own echo sentences",
    "scripts/shell-wfi-falsify-2776.sh":
        "2.776's offline falsifier for this suite's `sleep 6`; cited by a "
        "doc-comment in shell-wfi-test.sh. Its `sleep 6` is a GUARD asserting "
        "the defect is still present in HEAD's copy, and its `timeout 240` "
        "bounds an arm — neither is a window any suite waits on",
    "scripts/qemu-window-falsify-2771.sh":
        "2.771's offline falsifier for the `timeout N qemu; grep` shape; cited "
        "by a doc-comment in pi-test.sh as of 2.783. THIRD instance of the same "
        "mention-as-run cost, and now predictable: naming a falsifier in the "
        "suite it grades buys one entry here. Its two qemu-timeout hits are its "
        "own arm-announcing `echo` lines describing the OLD shape — prose about "
        "a window, not a window",
    "scripts/speaktext-probe-3263.sh":
        "3.643 — the FIFTH mention-as-run instance, and the first where the "
        "mention is STALE PROSE rather than a deliberate provenance line. "
        "speaktext-test.sh:15 says \"It CALLS scripts/speaktext-probe-3263.sh, "
        "unedited\" — written at 3.263 and never updated — while the line that "
        "actually decides is :111, `PROBE=\"scripts/speaktext-probe-3376.sh\"`. "
        "Measured, not inferred: 3376 mentions 3263 only in its own header "
        "comment and invokes nothing of it, and no other registered suite "
        "names it (the two remaining referrers, longspeak-probe-3369.sh:94 and "
        "preflight-falsify-3357.sh:20, are a comment and a falsifier default). "
        "So all seven of its SETTLE constants were charged to a file the walk "
        "never executes. Its live successor 3376 is NOT excused here — it is "
        "run, and its same seven constants are ruled by COUNT_NAMES below",
    "scripts/fixed-window-census.py":
        "THIS FILE. It entered its own driver class when shell-wfi-test.sh's "
        "2.776 comment cited the table that rules its driver windows — the "
        "mention-as-run defect pointed at the tool that found it. Every window "
        "measured in it is prose inside a string in this very table",
    "scripts/inuse-control-3639.sh":
        "3.669 — the SIXTH mention-as-run instance. tcp-inuse-test.sh:29 names "
        "it in a provenance comment (\"3.642 ALSO built scripts/inuse-control-"
        "3639.sh, inuse-selftest-3639.sh ... to prove the branch is "
        "differential\") explaining why that proof is NOT re-run inside the "
        "registered suite — a deliberate line, like qemu-window-falsify-2771.sh's, "
        "not stale prose. Measured, not inferred: no registered suite's PROBE=/ "
        "invocation line names inuse-control-3639.sh — only inuse-selftest-"
        "3639.sh (itself unregistered) mentions it besides this comment. Its own "
        "`port_free()` settimeout(3) is the same liveness-poll shape ruled for "
        "inuse-boot-probe-2859.sh above, charged to a file the walk never runs",
}

# ------------------------------------------------- the UNITS INVARIANT (3.643)
# THE SIXTH MISCOUNT, AND THE FIRST THAT OVERSTATES.
#
# Every earlier correction in this file's history ran the same way: the class
# was bigger than the last count said, in the flattering direction. This one
# runs the other way. `var-secs` decides a constant is SECONDS from the SHAPE
# OF ITS NAME (the stem list at :132 contains SETTLE), and speaktext-probe-
# 3376.sh's read barrier holds three SETTLE_* constants of which only ONE is a
# duration:
#
#     SETTLE_DT  = 0.25  seconds between samples   -> `sleep "$SETTLE_DT"` :236
#     SETTLE_K   = 3     consecutive EQUAL samples -> `[ "$run" -ge "$SETTLE_K" ]`
#     SETTLE_MAX = 40    hard sample CAP           -> `[ "$n" -lt "$SETTLE_MAX" ]`
#
# `n` and `run` are integer counters advanced by `n=$((n+1))`. So the census
# reported 40.00s for a number that buys 40 x 0.25 = 10 s, and 3.00s for a
# number that is not time at all — and it reported them SEVEN times per file,
# because five of the hits are the barrier's own offline selftest arms
# (`SETTLE_MAX=12 sb_arm ...`) re-arming the cap per fixture.
#
# WHY THIS IS A TABLE OF NAMES AND NOT SEVEN SENTENCES. EXCLUDED_DRIVERS keys
# on the reported source LINE, and 2.775 already wrote down what that costs:
# re-word the line and the census re-drifts, and the repair is to edit a table
# rather than to learn anything. Five of these seven lines carry an arm LABEL
# in prose. Keying on the NAME survives every one of those re-wordings.
#
# WHY IT IS NOT A NEW REGEX. Demoting the stem itself, or demoting any name
# never handed to a wait call, would also demote GUEST_SECS / PEER_SECS — which
# are genuinely seconds, are genuinely counted, and are ruled BUDGETS one table
# up. A rule that silences those is the flattering direction this class keeps
# failing in. So the demotion is named one at a time, per file, and printed
# every run.
#
# AND IT IS REVOCABLE BY MEASUREMENT, not by trust. Each entry is honoured only
# while wait_positioned() finds the name in NO wait position in that same file.
# Hand SETTLE_MAX to a `sleep`/`timeout`/`read -t` and the entry is REVOKED, the
# hits are counted again, and the drift arm names it. That is the invariant
# 2.775 asked for: the table states a claim about the file, and the claim is
# re-checked on every run instead of being taken on the date it was written.
COUNT_NAMES = {
    ("scripts/speaktext-probe-3376.sh", "SETTLE_K"):
        "a COUNT, not seconds: the number of consecutive EQUAL non-zero size "
        "samples that make a capture stationary, compared at "
        "`[ \"$run\" -ge \"$SETTLE_K\" ]` against a counter advanced by "
        "`run=$((run+1))`. The seconds in this barrier are SETTLE_DT=0.25",
    ("scripts/speaktext-probe-3376.sh", "SETTLE_MAX"):
        "a COUNT, not seconds: the hard SAMPLE cap, compared at "
        "`[ \"$n\" -lt \"$SETTLE_MAX\" ]` against `n=$((n+1))`. Its worth in "
        "time is SETTLE_MAX x SETTLE_DT = 10 s at the default, and reaching it "
        "REFUSES (rc 1 -> `STILL CHANGING at deadline`, sb_bad=1) rather than "
        "grading what it has — a named failure, so a ceiling even in seconds",
}

# The wait positions a shell constant can occupy. Deliberately generous: a name
# that reaches ANY of them is treated as seconds and its COUNT_NAMES entry is
# revoked, because a false revocation only re-counts a constant that is already
# printed, while a missed one hides time.
WAIT_POSITION = (r"(?:\bsleep|\btimeout|\bread\s+-t|\s-t)\s+"
                 r"[\"']?\$\{?%s\b")

def wait_positioned(path, name):
    """Is <name> ever handed to a wait call in this file? Re-measured per run."""
    try:
        text = io.open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return False
    return re.search(WAIT_POSITION % re.escape(name), text) is not None

# Only the two NAME= dialects can be demoted: the others match an expression,
# not a binding, so there is no name to rule.
COUNTABLE_DIALECTS = ("var-secs", "py-const-secs")
NAME_BINDING = re.compile(r"^\s*([A-Z_][A-Z0-9_]*)\s*=")

def count_name_of(rel, label, src):
    """The COUNT_NAMES key this hit is demoted by, or None."""
    if label not in COUNTABLE_DIALECTS:
        return None
    m = NAME_BINDING.match(src)
    if not m:
        return None
    key = (rel, m.group(1))
    return key if key in COUNT_NAMES else None

# The driver class as ruled in docs/audit-2.773-driver-windows.md.
# ONE member today. When scripts/screendump-patch-2772.py --in-place lands,
# this entry SHOULD drift — the patch moves the constant inside a try/except
# whose failure costs the screenshot and nothing else — and the drift is the
# point: it forces the audit-doc line that records the demotion to a ceiling.
KNOWN_DRIVERS = {
    # 3.667: this used to key on the py-timeout-kw dialect at the
    # `create_connection(..., timeout=10)` call. The 3.667 fix passes
    # `timeout=max(0, deadline - time.time())` instead (the REMAINING budget,
    # not a bare literal), so no dialect regex matches that call any more --
    # the same window is now expressed one line earlier and picked up under
    # py-deadline instead. Same window, same 10 s, same 675-suite exposure;
    # only the dialect and the keyed source line moved. Ruled in the 3.667
    # amendment to docs/audit-2.773-driver-windows.md.
    ("scripts/shell-demo.py", "py-deadline",
     'deadline = time.time() + 10'),
    # KNOWN BLIND SPOT, DECIDED NOT FIXED (3.669, docs/audit-2.773-driver-windows.md
    # gap (d)). `py-deadline` requires the literal text `time.time()` immediately
    # before the `+`. The 3.667 tick DRAFTED, then REJECTED, a variable-anchored
    # spelling of this exact line -- `t_open = time.time(); deadline = t_open + 10`
    # -- specifically because it measured that spelling as INVISIBLE to every
    # dialect here (py-deadline needs the literal call on the line; var-secs/
    # py-const-secs need an UPPERCASE-ish NAME at the start of the line; neither
    # matches `deadline = t_open + 10`). The line above is the REJECTED draft's
    # replacement, so today's tree has ZERO occurrences of the blind spelling.
    # Closing it for real needs a second binding pass (like bindings() below, but
    # tracking "NAME was assigned time.time()" rather than "NAME was assigned a
    # numeric literal") threaded through scan() as a second lookup table -- real
    # new machinery, for one call site that does not currently exist in the tree.
    # Judged not worth it while that is true: this comment is the tripwire. If a
    # future edit reintroduces `X = time.time()` followed by `... = X + N` as a
    # grading window, IT WILL NOT BE COUNTED -- read this block before trusting
    # a "driver class unchanged" report against such a diff.
    # The four below became visible only when py-const-secs was added (2.773).
    # Every one of them is a WINDOW by this file's own definition, because
    # shell-demo.py FALLS THROUGH on expiry instead of failing: `wait_ready()`
    # is called at :182 with its return value discarded, and the command loop
    # returns to the next command when its read expires. The suite then grades
    # whatever arrived — which is the class signature, a harness fault reported
    # under the kernel's name.
    ("scripts/shell-demo.py", "py-const-secs",
     'BOOT_CEILING = float(os.environ.get("AFRISH_BOOT_CEILING", "90"))'),
    # BOOT_QUIET is not a theoretical member: it red-ed `rjust` at row ~289 of
    # the live 2.770 walk while this census was being extended. Three seconds
    # of total silence after attach is read as "already booted", the driver
    # types, and typing into a still-booting guest wedges it — the driver's own
    # docstring (:169-176) measured that wedge and named its signature, "boot
    # dies at the preemption demo", which is exactly where .rat-logs-2770/
    # rjust.out stops. Distinguished from the 90 s path by evidence, not
    # assumption: `no prompt within 90s` appears in ZERO logs of this walk.
    ("scripts/shell-demo.py", "py-const-secs",
     'BOOT_QUIET = float(os.environ.get("AFRISH_BOOT_QUIET", "3"))'),
    # BOOT_GRACE arrived with 2.782, the fix for the line above, and it is a
    # WINDOW by the same test — expiry types into the guest rather than failing.
    # It is ruled a member and NOT an exclusion on purpose: it is the seconds a
    # guest gets to prove it is still booting, so if it is ever too short the
    # symptom is exactly BOOT_QUIET's, and this class is where that gets found.
    # What changed is the CONSEQUENCE, not the membership: the decision it ends
    # is retractable (any byte inside the window cancels it and costs nothing)
    # and both of its exits now print a line naming themselves, so a future
    # `rjust` is attributable from its own capture instead of by elimination
    # across a walk directory. Measured exposure, from the 2.770 walk rather
    # than from the suite count: 657 of 665 captures hold `AfriSh ready`, so
    # 98.8% of them saw boot output and cannot reach this branch at all.
    ("scripts/shell-demo.py", "py-const-secs",
     'BOOT_GRACE = float(os.environ.get("AFRISH_BOOT_GRACE", "6"))'),
    ("scripts/shell-demo.py", "py-const-secs",
     'CMD_QUIET = float(os.environ.get("AFRISH_CMD_QUIET", "30"))'),
    ("scripts/shell-demo.py", "py-const-secs",
     'CMD_CEILING = float(os.environ.get("AFRISH_CMD_CEILING", "90"))'),
    # ---- comparison-shaped, found when py-elapsed-cmp was added in 2.779 ----
    # Ruled in docs/audit-2.779-comparison-shaped-windows.md. All five are
    # WINDOWS, and by the strict test rather than by resemblance: expiry does
    # not END a wait, it PUTS A FRAME ON THE WIRE that the guest never asked
    # for. A ceiling that expires reports a failure; these report nothing and
    # change what the guest sees, which is how :516 turned a correct kernel
    # answer (RFC 793's SYN/ACK to a repeated SYN) into row 681's red in the
    # 2.776 walk, by 218 ms — docs/audit-2.777-tcp-serve2-red.md.
    #
    # The rto pair (:521, :446) is ruled a MEMBER rather than excluded, and
    # that is the conservative direction on purpose: 2.778 repaired the VERDICT
    # that :516's spurious frame tripped, not the spuriousness, and no
    # measurement here says a spurious DATA retransmit is harmless. Excluding
    # it would need that measurement. "Never delete a line to make the number
    # smaller" cuts both ways.
    ("scripts/tcp-serve2-peer.py", "py-elapsed-cmp",
     "and time.time() - c.dialed > retry_after and c.dials < syn_tries: "
     "[retry_after=1.5]"),
    ("scripts/tcp-serve2-peer.py", "py-elapsed-cmp",
     "if c.inflight and time.time() - c.inflight[2] > rto and c.inflight[3] < 4: "
     "[rto=1.5]"),
    ("scripts/tcp-chat-peer.py", "py-elapsed-cmp",
     "if dialed is not None and not syn_acks and time.time() - dialed > "
     "retry_after and dials < 2: [retry_after=4]"),
    ("scripts/tcp-chat-peer.py", "py-elapsed-cmp",
     "if inflight and time.time() - inflight[2] > rto and inflight[3] < 4: "
     "[rto=1.5]"),
    # Same window as tcp-chat-peer.py:439 at the same 4 s, spelled
    # `float(sys.argv[4]) if len(sys.argv) > 4 else 4.0` — visible only because
    # bindings() reads an IfExp's else branch. Before that it sat in the
    # UNRESOLVED list, i.e. found and uncounted, which is the half-fix this
    # dialect was written to avoid.
    ("scripts/tcp-dial-peer.py", "py-elapsed-cmp",
     "if dialed is not None and not syn_acked and time.time() - dialed > "
     "retry_after and dials < 2: [retry_after=4]"),
}


def key_of(rel, label, src):
    """Ruled-table key: path + dialect + the reported source line with runs of
    whitespace collapsed. Line numbers are NOT part of it — a patch that moves
    a constant has not changed it."""
    return (rel, label, " ".join(src.split()))

# The class as ruled in docs/audit-2.770-fixed-window-suites.md §7. `--check`
# fails when the measured set differs from this — either a new member appeared
# or one was repaired; both want a line in the audit doc before this set moves.
# KNOWN is a SET, not a dict -- `{}` with no elements parses as an empty DICT
# in Python, and `measured - KNOWN` (a set minus a dict) raises TypeError. The
# 3.701 edit that emptied this table (below) left the bare `{...}` literal and
# never re-ran `--check` to notice; `set()` is the only spelling that stays a
# set once its last member (`uart-rx-stress`) is gone. If a future tick adds a
# member back, either spelling works -- `{"x"}` is a set literal same as before.
KNOWN = set()
# `dhcp` and `config` LEFT this set in 2.789: 2.788/2.788d converted both to
# the `pi` shape, so their only remaining qemu-timeout hit is the backstop
# ruled in EXCLUDED. They move to FIXED below rather than being deleted, so
# a revert is still legible by name.
# `gicbase` (2.951) and `frame-bounds` (2.952) LEFT this set: both Part B
# ceilings became `afrish_wait_markers "$B" 60` (gicbase-test.sh:64,
# frame-bounds-test.sh:81), the same conversion `reboot`/`shell-wfi` took —
# and unlike pi/dhcp/config/smpinfo they keep NO excluded backstop, so
# scan() returns [] outright. They move to FIXED below, where a reverted
# seam trips REGRESSED by name.
# `persist` and `persist-mtime` LEFT it at 3.130 for a DIFFERENT reason and
# must not be read as repaired: nothing was converted (both still hand
# `AFRISH_BOOT_GRACE=10` to shell-demo.py), the 10 s was ruled a CONDITIONAL
# hold that 0 of 32 recorded runs entered (3.129) and moved COUNTED ->
# EXCLUDED. They go to EXCUSED below, not FIXED.
# `uart-rx-stress` LEFT this set at 3.701: its last COUNTED constant, the
# per-trial `end = time.time() + 3.0` at :177, was MEASURED (a standalone
# probe replaying the identical shape found 300/300 trials answered well
# inside the ceiling, 2.53 s of margin) and ruled EXCLUDED/py-deadline,
# conditional on the same per-trial early-break text (SEAM_REQUIRED
# `echo-prompt-break`). scan("scripts/uart-rx-stress-test.sh") still returns
# hits (py-timeout-kw, py-elapsed-cmp, py-deadline) — like pi/dhcp/config/
# smpinfo, not like gicbase/frame-bounds — so it moves to FIXED below with a
# backstop still ruled, and any of its three seams breaking trips REGRESSED.
# `smpinfo` LEFT this set in 2.869: Part B's `sleep 12; kill; grep` became
# `timeout 60 qemu &; afrish_wait_markers; kill; ONE snapshot`, the fourth suite
# to take the `pi` shape, so its only remaining qemu-timeout hit is the backstop
# ruled in EXCLUDED and pinned in EXCLUDED_COVERS. It moves to FIXED below
# rather than being deleted, so a revert is still legible by name.
# Repaired members, kept by name so a REGRESSION is legible: `smoke` and `smp`
# gated on `sleep 8`/`sleep 12` until 2.770 replaced both with
# `afrish_wait_marker` (scripts/lib/afrish-boot.sh). If either reappears above,
# that seam was reverted.
# `reboot` joined them in 2.775: reboot-test.sh:37/:41 were `drain(9)` twice —
# a hard 9 s per boot phase with NO fall-through, the only member measured to
# red a suite in the live 2.770 walk (row 42, `shell never came up in boot 1`,
# docs/audit-2.774-walk-reds-vs-census.md §3). Both are now
# `wait_for(pred, AFRISH_BOOT_CEILING, ...)`: the guest's own banner ends the
# wait, the cap only bounds it. scan("scripts/reboot-test.sh") returns [].
# `shell-wfi` joined them in 2.776: :16 was `sleep 6 # let the shell idle-loop
# accrue wfi's before we type`, a HOST wait standing in for GUEST progress, and
# the second of the two class members measured to red the live 2.770 walk (row
# 34, `idle-wfi is 0; shell not sleeping` at `timer 12 ticks` — the guest had
# run ~120 ms inside that 6 s). The silence is now shell-demo.py's bare ":"
# payload, entered only after the prompt exists, and the suite grades the guest
# TICK DELTA across it, so a starved guest is reported as starved instead of as
# a console that refuses to sleep. scan("scripts/shell-wfi-test.sh") returns [].
# `pi` joined them in 2.783, and it is the first of the fourth dialect
# (qemu-timeout) to be repaired: `timeout 6 qemu ...; grep` became
# `timeout 60 qemu ... &` + `afrish_wait_markers` (the PLURAL seam — the raspi3b
# guest never stops and its six asserted lines arrive interleaved, so no single
# line can be nominated "the last") + `kill` + ONE snapshot. Unlike `reboot` and
# `shell-wfi`, scan("scripts/pi-test.sh") does NOT return [] — the backstop
# `timeout 60` is still there and is ruled in EXCLUDED above with its reason.
# `dhcp` (2.788c) and `config` (2.788d) then took the same shape, which is why
# this dialect has no KNOWN members left. All three are excused CONDITIONALLY —
# see SEAM_REQUIRED above: delete the seam call and the window is not silenced,
# it lands back in `measured`, which for these three ALSO trips REGRESSED below.
FIXED = {"smoke", "smp", "reboot", "shell-wfi", "pi", "dhcp", "config", "smpinfo",
         "gicbase", "frame-bounds", "uart-rx-stress"}

# EXCUSED is NOT a second spelling of FIXED. A FIXED member's window is GONE
# from the tree (converted to a marker wait); an EXCUSED member's window is
# still written, still handed to the guest, and is silent only because this
# census RULED it out in EXCLUDED. Parking `persist`/`persist-mtime` in FIXED
# would claim a repair that was never made — and it would be unfalsifiable
# there, because an EXCLUDED hit never reaches `measured`, so `FIXED & measured`
# could not fire for them no matter what the suite did. The membership that IS
# checkable is the exclusion itself: delete the EXCLUDED pin and the 10 s lands
# back in `measured`, which the arm below reports by name (and `added` reds).
EXCUSED = {"persist", "persist-mtime"}


def registry():
    src = open(os.path.join(ROOT, "scripts", "run-all-tests.sh"), encoding="utf-8").read()
    names = re.search(r"^SUITES=\((.*?)\)\s*$", src, re.S | re.M).group(1).split()
    return names, {n: i + 1 for i, n in enumerate(names)}


def pystrip(text):
    """Blank out python comments and DOCSTRINGS, KEEPING line numbers.

    Reason this exists, measured: scanning scripts/shell-demo.py raw reports
    `sh-sleep :134 8.00s` — which is the driver's own docstring explaining that
    464 suites gate on a `sleep 8` guess. Reporting prose as a member is the
    hand-count failure this file was written to end, so the driver pass reads
    CODE only. Blanking (rather than deleting) keeps every reported line number
    equal to the number in the editor.

    DOCSTRINGS, NOT ALL STRINGS — and this file's first version got that wrong,
    in the flattering direction, for about twenty minutes. Blanking every
    string literal also blanks `os.environ.get("AFRISH_BOOT_QUIET", "3")`,
    because a python default is a string; the census then reported shell-demo.py
    as having ONE window when it has five, and the one it hid is the 3 s quiet
    fall-through that red-ed `rjust` in the live walk while this was being
    written. A scanner whose own defence hides constants is worse than no
    scanner: it prints a small number with authority.

    A file that will not parse is returned unchanged and says so, because
    silently scanning nothing is the flattering direction too.
    """
    try:
        toks = list(tokenize.generate_tokens(io.StringIO(text).readline))
    except (tokenize.TokenError, IndentationError, SyntaxError) as e:
        print("  !! pystrip: %s — scanning raw text (comments included)" % e)
        return text
    prose = set()                       # line numbers occupied by a docstring
    try:
        import ast
        tree = ast.parse(text)
        for node in ast.walk(tree):
            body = getattr(node, "body", None)
            if not isinstance(body, list):   # IfExp/Lambda carry a node here
                continue
            for stmt in body:
                if (isinstance(stmt, ast.Expr)
                        and isinstance(stmt.value, ast.Constant)
                        and isinstance(stmt.value.value, str)):
                    prose.update(range(stmt.lineno,
                                       (stmt.end_lineno or stmt.lineno) + 1))
    except (SyntaxError, ValueError) as e:
        print("  !! pystrip: %s — docstrings not identified" % e)
    lines = text.splitlines()
    for tok in toks:
        if tok.type == tokenize.STRING and tok.start[0] not in prose:
            continue                    # a literal doing real work, e.g. an
        if tok.type not in (tokenize.COMMENT, tokenize.STRING):  # env default
            continue
        (r1, c1), (r2, c2) = tok.start, tok.end
        for r in range(r1, r2 + 1):
            if r - 1 >= len(lines):
                break
            line = lines[r - 1]
            a = c1 if r == r1 else 0
            b = c2 if r == r2 else len(line)
            lines[r - 1] = line[:a] + " " * (b - a) + line[b:]
    return "\n".join(lines)


# `py-const-secs` is python assignment syntax and is applied to `.py` files
# ONLY. Measured reason: run against `.sh` it also matches `GUEST_SECS=30`,
# which `var-secs` already reports and EXCLUDED already rules — the same
# constant under a second label reads as a NEW member, and it moved the ruled
# eleven to thirteen (`tcp-serve`, `tcp-serve2`) on its first run. A dialect
# that renames existing findings is not a census, it is double counting.
PY_ONLY = {"py-const-secs"}

# Dialects where the constant governs a CALL that can raise — only these have a
# meaningful guard column. A constant DEFINITION does not expire, so claiming
# "expiry propagates" about it would be theatre. Nor does a COMPARISON: it is
# read by a poll loop that is already running, so there is nothing to catch —
# which is worth saying in its own words rather than borrowing "definition".
GUARDABLE = {"py-timeout-kw", "py-settimeout"}
NOT_GUARDABLE_WHY = {"py-elapsed-cmp": "n/a (comparison — nothing raises)"}

CATCHERS = {"OSError", "socket.error", "socket.timeout", "TimeoutError",
            "Exception", "BaseException"}


def guards(path):
    """Measure, per line, whether an expiring timeout on that line would be
    CAUGHT — i.e. which `try:` bodies the line sits in and what they handle.

    This is the column 2.772 turned on. `shell-demo.py:71` and `:200` are the
    same dialect at the same value in the same file; the only thing that makes
    one a ceiling and the other the constant that deleted a suite's evidence is
    that :71's expiry is handled and :200's propagates out of main(). Written
    reasons already claimed that. This derives it from the syntax tree, so the
    claim and the measurement are independent — and when
    scripts/screendump-patch-2772.py --in-place lands, this column changes by
    itself instead of waiting for someone to remember to edit prose.

    Returns {lineno: "caught by except OSError@72" | ""}. Membership is still
    ruled by the tables above: being caught is NECESSARY for a constant to be a
    ceiling, not sufficient (a handler that exits 1 still reds the suite), so
    the census prints this beside the ruling rather than replacing it.
    """
    import ast
    out = {}
    try:
        tree = ast.parse(open(path, encoding="utf-8", errors="replace").read())
    except (SyntaxError, ValueError) as e:
        print("  !! guards: %s — guard column unavailable for %s" % (e, path))
        return out

    def names(handler):
        t = handler.type
        if t is None:
            return "bare except"
        parts = [t] if not isinstance(t, ast.Tuple) else list(t.elts)
        got = []
        for p in parts:
            try:
                got.append(ast.unparse(p))
            except Exception:
                got.append("?")
        return "except " + ", ".join(got)

    for node in ast.walk(tree):
        if not isinstance(node, ast.Try):
            continue
        catching = [h for h in node.handlers
                    if h.type is None
                    or any(n in CATCHERS
                           for n in (ast.unparse(h.type).replace("(", " ")
                                     .replace(")", " ").replace(",", " ").split()))]
        if not catching:
            continue
        label = "caught by %s@%d" % (names(catching[0]), catching[0].lineno)
        for body in (node.body, node.orelse):
            for stmt in body:
                lo = stmt.lineno
                hi = getattr(stmt, "end_lineno", lo) or lo
                for ln in range(lo, hi + 1):
                    out.setdefault(ln, label)
    return out


# Every candidate whose constant could not be resolved to a number, so that a
# comparison-shaped window is never dropped in silence: (rel, label, lineno,
# name, why, src). Refilled by main(); appended to by scan().
UNRESOLVED = []

SH_NUM_ASSIGN = re.compile(r"^\s*([A-Za-z_]\w*)\s*=\s*([0-9]+(?:\.[0-9]+)?)\s*(?:#|$)")


def bindings(text, is_py):
    """name -> (seconds | None, why-it-is-None), for the names a
    comparison-shaped window compares against.

    TUPLE UNPACKING IS NOT AN EDGE CASE HERE — it is the shape of the defect
    this resolver was written for: `retry_after, rto, syn_tries = 1.5, 1.5, 8`
    binds all three constants of tcp-serve2-peer.py on one line, and a resolver
    that only read `name = literal` would have resolved none of them and left
    the 2.777 window in the UNRESOLVED list, i.e. found and not counted.

    FUNCTION PARAMETER DEFAULTS ARE DELIBERATELY NOT RESOLVED, and the reason
    was MEASURED after a weaker one had already been written here. The weak
    version said "resolving `quiet` at shell-demo.py:177 would double-count
    BOOT_QUIET, which py-const-secs already rules at :54" — true about double
    counting, and not actually the reason that site is safe: BOOT_QUIET is bound
    to `float(os.environ.get(...))`, a CALL, so it never enters this map and a
    parameter-reading resolver still could not reach 3.0 through it.

    Building the resolver that CAN (env-string default, then parameter default)
    showed the real hazard, and it is worse than a duplicate: this map is
    FILE-scoped and a parameter is not. shell-demo.py defines `quiet` TWICE —
    `drain_until_prompt(start, quiet=CMD_QUIET…)` at :99 and
    `wait_ready(…, quiet=BOOT_QUIET)` at :130 — so the two-hop resolver bound
    the name to the first one it walked and reported :177, a 3 s window, as a
    30 s one. A wrong number carries the census's authority exactly as far as a
    right one. Names smaller in scope than the file are therefore not resolved
    here at all; the use site is printed as UNRESOLVED, so the refusal is
    visible on every run instead of being a silent skip.

    AMBIGUITY IS REPORTED, NOT PICKED. A name bound to two DIFFERENT literals
    resolves to None with its bindings named; only agreeing bindings collapse.
    A binding to a non-literal (`retry_after = float(argv[i + 1])`, the CLI
    override at :439) is not a binding at all for this purpose — it is why the
    number is a default rather than a constant, and the report says so.
    """
    seen = {}                      # name -> {value: first lineno}
    if is_py:
        import ast
        try:
            tree = ast.parse(text)
        except (SyntaxError, ValueError) as e:
            print("  !! bindings: %s — names cannot be resolved in this file" % e)
            return {}
        for node in ast.walk(tree):
            if not isinstance(node, ast.Assign):
                continue
            for tgt in node.targets:
                pairs = []
                if isinstance(tgt, ast.Name):
                    pairs = [(tgt, node.value)]
                elif (isinstance(tgt, (ast.Tuple, ast.List))
                        and isinstance(node.value, (ast.Tuple, ast.List))
                        and len(tgt.elts) == len(node.value.elts)):
                    pairs = list(zip(tgt.elts, node.value.elts))
                for name_node, val in pairs:
                    if not isinstance(name_node, ast.Name):
                        continue
                    # `X = <argv> if <argc> else 4.0` is the SAME binding as
                    # `X = 4.0` with a CLI override — measured on the tree:
                    # tcp-dial-peer.py:124 spells its 4 s exactly that way and
                    # tcp-chat-peer.py:320 spells the identical window as a
                    # tuple literal. Resolving one and not the other would make
                    # the census report a window's seconds or "unresolved"
                    # according to spelling, which is the defect this dialect
                    # exists to end.
                    if isinstance(val, ast.IfExp):
                        val = val.orelse
                    if (isinstance(val, ast.Constant)
                            and isinstance(val.value, (int, float))
                            and not isinstance(val.value, bool)):
                        by = seen.setdefault(name_node.id, {})
                        by.setdefault(float(val.value), val.lineno)
    else:
        for lineno, line in enumerate(text.splitlines(), 1):
            m = SH_NUM_ASSIGN.match(line)
            if m:
                seen.setdefault(m.group(1), {}).setdefault(float(m.group(2)), lineno)
    out = {}
    for name, by in seen.items():
        if len(by) == 1:
            out[name] = (next(iter(by)), "")
        else:
            out[name] = (None, "%d differing literal bindings (%s)" % (
                len(by), ", ".join("%g@:%d" % (v, ln)
                                   for v, ln in sorted(by.items()))))
    return out


def scan(path):
    hits = []
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return hits
    is_py = path.endswith(".py")
    # Bindings come off the RAW text: pystrip() blanks a docstring to spaces,
    # and a function whose whole body is a docstring then has no body left to
    # parse. Constants are code either way, so nothing is lost by reading them
    # before the prose defence runs.
    binds = bindings(text, is_py)
    rel = os.path.relpath(path, ROOT).replace("\\", "/")
    if is_py:
        text = pystrip(text)
    for lineno, line in enumerate(text.splitlines(), 1):
        if line.strip().startswith("#"):
            continue
        for label, rx, _ in DIALECTS:
            if label in PY_ONLY and not is_py:
                continue
            for m in rx.finditer(line):
                src = line.strip()[:120]
                token = m.group(1)
                try:
                    value = float(token)
                except ValueError:
                    value, why = binds.get(token, (None, "no numeric binding "
                                                         "in this file"))
                    if value is None:
                        UNRESOLVED.append((rel, label, lineno, token, why, src))
                        continue
                    # The resolved constant joins the KEY on purpose: moving the
                    # comparison must not read as drift, changing its seconds
                    # must.
                    src = "%s [%s=%g]" % (src, token, value)
                if value >= DIALECT_THRESHOLD.get(label, THRESHOLD):
                    hits.append((label, lineno, value, src))
    return hits


def walk_reds(path):
    """Non-PASS rows of a runner results TSV: [(name, status, secs)].

    The runner writes name<TAB>status<TAB>secs<TAB>rc<TAB>epoch with NO header
    row, so every line is data — measured on .rat-results.2770.tsv, whose first
    line is `smoke\tPASS\t30\t0\t...`. Skipping line 1 as a header would drop a
    real suite, so nothing here is skipped by position.

    Safe to read while a walk is still writing: the runner appends a row at a
    time, and a torn final line has too few columns and is skipped rather than
    guessed at.
    """
    reds = []
    try:
        fh = open(path, encoding="utf-8", errors="replace")
    except OSError as exc:
        print("  !! cannot read walk results %r: %s" % (path, exc))
        return reds
    with fh:
        for line in fh:
            col = line.rstrip("\n").split("\t")
            if len(col) < 3:
                continue
            if col[1] != "PASS":
                reds.append((col[0], col[1], col[2]))
    return reds


DRIVER_REF = re.compile(r"(?:scripts/|\blib/)([A-Za-z0-9_.-]+\.(?:py|sh))")


def drivers(names):
    """Which shared files do the registered suites actually run, and how many
    suites run each? Measured by reading every suite file — the 675 in the
    module docstring is this function's output, not a remembered number.

    A file that is itself a registered suite is skipped: it is already covered
    by the suite pass, and counting it twice would inflate both populations.
    """
    suite_files = {n + "-test.sh" for n in names}
    users = {}
    for name in names:
        path = os.path.join(ROOT, "scripts", name + "-test.sh")
        if not os.path.exists(path):
            continue
        text = open(path, encoding="utf-8", errors="replace").read()
        for m in DRIVER_REF.finditer(text):
            fn = m.group(1)
            if fn in suite_files:
                continue
            for cand in (os.path.join(ROOT, "scripts", fn),
                         os.path.join(ROOT, "scripts", "lib", fn)):
                if os.path.exists(cand):
                    rel = os.path.relpath(cand, ROOT).replace("\\", "/")
                    users.setdefault(rel, set()).add(name)
                    break
    return users


def main():
    # 2.791 (master side; this file was reconciled with the orphan branch at
    # 2.801 and this block is the half master brings): the report prints `⇒` in
    # the driver-exclusion reasons. Python 3.14 picks the ANSI codepage (cp1252
    # on this box) for a NON-TTY stdout, so `python scripts/fixed-window-census.py
    # > file` died at the exclusion table with UnicodeEncodeError — AFTER the
    # whole suite half had printed, so the file read like a clean report carrying
    # rc 1. Every exit code taken from a redirected run was therefore an encoding
    # artefact, not a census verdict. Wrap stdout/stderr in UTF-8 so the script
    # reports its OWN verdict whether it is piped, redirected or on a terminal.
    # NOTE this now matters MORE than it did at 2.791: the seam-invariant block
    # below prints `NO — SEAM GONE`, an em-dash on the one line that announces a
    # broken invariant, so without this the failure report is the part that dies.
    for _stream in ("stdout", "stderr"):
        _s = getattr(sys, _stream)
        # `.encoding` EXISTS but is None on the selftest's capture stream, so the
        # 3-arg getattr default never fires — `or ""` is the arm that matters.
        if _s is not None and (getattr(_s, "encoding", None) or "").lower() \
                not in ("utf-8", "utf8"):
            try:
                _s.reconfigure(encoding="utf-8")
            except (AttributeError, io.UnsupportedOperation, ValueError):
                pass  # exotic stream: leave it alone rather than lose the report

    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true",
                    help="exit 1 if either measured class differs from KNOWN")
    ap.add_argument("--vs-walk", metavar="TSV",
                    help="grade the ruled class against a walk's actual reds "
                         "(a runner --results TSV); with --check, a red that "
                         "has NO ruled window on its path exits 1")
    args = ap.parse_args()

    names, row = registry()
    del UNRESOLVED[:]
    print("registry: %d suites (scripts/run-all-tests.sh)" % len(names))
    print("threshold: >= %.0f s   dialects: %s" % (THRESHOLD, ", ".join(d[0] for d in DIALECTS)))
    print("threshold overrides: %s" % ", ".join(
        "%s >= %g s" % kv for kv in sorted(DIALECT_THRESHOLD.items())))

    found, excluded_seen, seam_lost = {}, [], []
    uncovered = []
    for name in names:
        path = os.path.join(ROOT, "scripts", name + "-test.sh")
        if not os.path.exists(path):
            print("  !! no suite file for registered name %r" % name)
            continue
        keep = []
        with io.open(path, encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        seam = {s: seam_present(text, s) for s in SEAMS}
        # One draw-down pool per (suite, dialect), per suite file. See
        # EXCLUDED_COVERS: an excuse covers the constants it was WRITTEN for,
        # not every constant of its dialect that ever appears beside them.
        pool = {}
        for hit in scan(path):
            reason = EXCLUDED.get((name, hit[0]))
            need = SEAM_REQUIRED.get((name, hit[0]))
            if reason and need and not seam[need]:
                # The excuse described a repair this file no longer contains.
                seam_lost.append((row[name], name, hit[0]))
                reason = None
            if reason and (name, hit[0]) in EXCLUDED_COVERS:
                key = (name, hit[0])
                if key not in pool:
                    pool[key] = collections.Counter(EXCLUDED_COVERS[key])
                if pool[key][hit[2]] > 0:
                    pool[key][hit[2]] -= 1
                else:
                    # Not what this excuse was written for — a NEW window, or
                    # one more of them than was ruled on. Do not silence it.
                    uncovered.append((row[name], name, hit[0], hit[1], hit[2]))
                    reason = None
            if reason:
                excluded_seen.append((row[name], name, hit[0], hit[2], reason))
            else:
                keep.append(hit)
        if keep:
            found[name] = keep

    print("\n=== FIXED-WINDOW CLASS: %d suites ===" % len(found))
    total = 0.0
    for name in sorted(found, key=lambda n: row[n]):
        secs = sum(h[2] for h in found[name])
        total += secs
        print("row %4d  %-16s %6.1fs total" % (row[name], name, secs))
        for label, lineno, value, src in found[name]:
            print("            %-14s :%-4d %6.2fs | %s" % (label, lineno, value, src))
    print("\nfixed seconds burned per full walk, this class: %.0f s" % total)

    print("\n=== EXCLUDED (ceilings/budgets, listed so they are never silent) ===")
    for r, name, label, value, reason in sorted(excluded_seen):
        print("row %4d  %-16s %-14s %6.2fs — %s" % (r, name, label, value, reason))

    print("\n=== COVER INVARIANT (an excuse covers only what it was written for) ===")
    for key in sorted(EXCLUDED_COVERS):
        pinned = EXCLUDED_COVERS[key]
        extra = [u for u in uncovered if (u[1], u[2]) == key]
        print("  %-16s %-14s pinned %-22s %s"
              % (key[0], key[1],
                 "/".join("%gs" % v for v in pinned) if pinned
                 else "(none — dead excuse)",
                 "NOT COVERED: %d" % len(extra) if extra else "all hits covered"))
    if uncovered:
        print("  !! UNCOVERED — these hits are of an excused dialect but are NOT "
              "the constants that excuse was written for. They are counted as "
              "FIXED members above:")
        for r, name, label, lineno, value in sorted(uncovered):
            print("     row %4d  %-16s %-14s :%-5s %.2fs" % (r, name, label, lineno, value))

    print("\n=== SEAM INVARIANT (a conditional exclusion states its condition) ===")
    for name, label in sorted(SEAM_REQUIRED):
        broken = any(s[1] == name and s[2] == label for s in seam_lost)
        print("  %-16s %-14s excused while %-44s : %s"
              % (name, label, SEAMS[SEAM_REQUIRED[(name, label)]][0],
                 "NO — SEAM GONE" if broken else "yes"))
    if seam_lost:
        print("  !! SEAM LOST — the exclusion above describes a repair these "
              "files no longer contain, so their windows are NOT silenced: %s"
              % sorted(set(s[1] for s in seam_lost)))

    measured = set(found)
    added, gone = measured - KNOWN, KNOWN - measured
    print("\nvs the ruled class in docs/audit-2.770-fixed-window-suites.md §7:")
    print("  new members not yet ruled on : %s" % (sorted(added) or "none"))
    print("  ruled members now clean      : %s" % (sorted(gone) or "none"))
    print("  repaired members, still clean: %s" % (sorted(FIXED - measured) or "none"))
    if FIXED & measured:
        print("  !! REGRESSED (afrish_wait_marker seam reverted?): %s"
              % sorted(FIXED & measured))
        added |= FIXED & measured
    print("  excused members, still silent: %s" % (sorted(EXCUSED - measured) or "none"))
    if EXCUSED & measured:
        print("  !! EXCUSED member is COUNTED again (its EXCLUDED pin is gone?): %s"
              % sorted(EXCUSED & measured))
        added |= EXCUSED & measured
    # seam_lost is its OWN drift term rather than only relying on the terms
    # above. As of 2.789 `dhcp` and `config` moved KNOWN -> FIXED, so a lost
    # seam for any of the three DOES trip REGRESSED — but that is a property of
    # which set they sit in TODAY. Move one back to KNOWN and a lost seam would
    # merely SHRINK `gone`, the one arithmetic that reads as things getting
    # better; this term is what makes the invariant independent of that choice.
    suite_drift = bool(added or gone or seam_lost or uncovered)
    if args.check and suite_drift:
        print("CENSUS DRIFT — update the audit doc, then update KNOWN here.")

    # ---------------------------------------------------------- driver pass
    # Printed AFTER everything above, so the suite half of this report is a
    # byte-identical prefix of what the census printed before drivers existed
    # (selftest arm 9 asserts exactly that against the committed version).
    used = drivers(names)
    file_excluded = sorted((rel, len(used[rel]), EXCLUDED_DRIVER_FILES[rel])
                           for rel in used if rel in EXCLUDED_DRIVER_FILES)
    used = {rel: u for rel, u in used.items() if rel not in EXCLUDED_DRIVER_FILES}
    print("\n=== SHARED DRIVERS: %d files the registered suites run ===" % len(used))
    d_found, d_excluded, d_windows = {}, [], {}
    d_counts, d_revoked = [], []          # 3.643 units invariant
    for rel in sorted(used, key=lambda r: -len(used[r])):
        n_users = len(used[rel])
        path = os.path.join(ROOT, rel)
        hits = scan(path)
        g = guards(path) if rel.endswith(".py") and hits else {}
        keep = []
        for label, lineno, value, src in hits:
            # 3.643 — a constant that is not TIME cannot be a window or a
            # ceiling; the window/ceiling question does not apply to it. The
            # demotion is revoked the moment the name reaches a wait call,
            # and a revoked hit falls through to be COUNTED, so the drift arm
            # is what reports it rather than this branch swallowing it.
            ckey = count_name_of(rel, label, src)
            if ckey is not None:
                if wait_positioned(path, ckey[1]):
                    d_revoked.append((rel, ckey[1], lineno, value, src))
                else:
                    d_counts.append((rel, ckey[1], lineno, value, src,
                                     COUNT_NAMES[ckey]))
                    continue
            reason = EXCLUDED_DRIVERS.get(key_of(rel, label, src))
            if reason:
                d_excluded.append((rel, label, lineno, value, src, reason))
            else:
                keep.append((label, lineno, value, src))
        d_windows[rel] = keep
        print("  %-38s run by %4d suites   windows: %d" % (rel, n_users, len(keep)))
        for label, lineno, value, src in keep:
            print("        %-14s :%-4d %6.2fs  exposure %d suites  guard=%s | %s"
                  % (label, lineno, value, n_users,
                     (g.get(lineno) or "NONE — expiry propagates")
                     if label in GUARDABLE
                     else NOT_GUARDABLE_WHY.get(label, "n/a (definition)"), src))
            d_found[key_of(rel, label, src)] = (lineno, value, n_users, g.get(lineno))
        for label, lineno, value, src in [h for h in hits
                                          if EXCLUDED_DRIVERS.get(key_of(rel, h[0], h[3]))]:
            if rel.endswith(".py"):
                print("        (excluded)     :%-4d %6.2fs  guard=%s"
                      % (lineno, value,
                         (g.get(lineno) or "NONE — expiry propagates")
                         if label in GUARDABLE else "n/a (definition)"))

    print("\n=== EXCLUDED in drivers (ceilings/budgets, never silent) ===")
    for rel, label, lineno, value, src, reason in sorted(d_excluded):
        print("  %-30s %-14s :%-4d %6.2fs — %s" % (rel, label, lineno, value, reason))
    for rel, n_users, reason in file_excluded:
        print("  %-30s WHOLE FILE (mentioned by %d suite(s), run by none) — %s"
              % (rel, n_users, reason))

    # ---------------------------------------------------- the units invariant
    # Printed as its own section and never folded into the exclusions above: an
    # exclusion says "this many seconds, and here is why they are not graded
    # against"; these lines say "this is not seconds". Collapsing the two would
    # put a ruled ceiling and a unit error under one heading, which is the kind
    # of tidy that costs a later reader the finding.
    print("\n=== NOT A DURATION (units invariant: named COUNTS, re-checked every run) ===")
    for rel, name, lineno, value, src in sorted(d_revoked):
        print("  !! REVOKED %-22s %-14s :%-4d — %s is in a WAIT position in this "
              "file, so its COUNT_NAMES entry does not hold and the hit is "
              "COUNTED: %s" % (rel, name, lineno, name, src))
    for rel, name, lineno, value, src, why in sorted(d_counts):
        print("  %-30s %-12s :%-4d (var-secs read it as %.2fs) — %s"
              % (rel, name, lineno, value, why))
    if not d_counts and not d_revoked:
        print("  none")

    # ---------------------------------------------------- unresolved candidates
    # A comparison-shaped window whose constant this scanner could not pin down.
    # It is NOT drift and it does NOT exit 1 — the census has not measured a
    # number, so it has nothing to rule. It is printed because the alternative
    # is a candidate dropped in silence, which is precisely how the 2.777
    # window survived four censuses.
    print("\n=== UNRESOLVED candidates (found, not counted — never silent) ===")
    for rel, label, lineno, name, why, src in sorted(set(UNRESOLVED)):
        print("  %-30s %-14s :%-4d %r %s | %s" % (rel, label, lineno, name,
                                                  "— " + why if why else "", src))
    if not UNRESOLVED:
        print("  none")

    d_measured = set(d_found)
    d_added, d_gone = d_measured - KNOWN_DRIVERS, KNOWN_DRIVERS - d_measured
    worst = max((v[2] for v in d_found.values()), default=0)
    print("\ndriver class: %d constant(s), worst exposure %d suites "
          "(the per-suite class is %d suites total)" % (len(d_found), worst, len(found)))
    print("vs the ruled driver class in docs/audit-2.773-driver-windows.md:")
    print("  new driver windows not yet ruled on: %s"
          % (sorted("%s %s | %s" % k for k in d_added) or "none"))
    print("  ruled driver windows now clean     : %s"
          % (sorted("%s %s | %s" % k for k in d_gone) or "none"))
    driver_drift = bool(d_added or d_gone)
    if args.check and driver_drift:
        print("DRIVER CENSUS DRIFT — rule it in the audit doc, then move "
              "KNOWN_DRIVERS/EXCLUDED_DRIVERS here.")

    # ------------------------------------------------------------ walk pass
    # Both passes above are STATIC: they rule a class out of the source and
    # rank it by exposure. Neither has ever been graded against a walk that
    # actually went red. This pass does that, and it is printed LAST so the
    # default report (no --vs-walk) stays byte-identical — selftest arm 9
    # compares the suite half against the previous committed census.
    #
    # It answers ONE question and must be read as answering only that one:
    # does every red in this walk have a ruled window somewhere on its path?
    # A red WITH a window is a candidate, never a proven cause — the cause of
    # any single red is established by reading that suite's log. A red with NO
    # window is the load-bearing output: it is the one that cannot be waved
    # away as contention, and it exits 1 so nobody has to notice it by eye.
    walk_drift = False
    if args.vs_walk:
        reds = walk_reds(args.vs_walk)
        print("\n=== WALK REDS vs THE RULED CLASS: %s ===" % args.vs_walk)
        print("a ruled window on the path is a CANDIDATE, not a cause; "
              "the cause of one red is read out of that suite's log.")
        unattributed = []
        for name, status, secs in reds:
            print("  %-16s %-6s %4ss" % (name, status, secs))
            hit = False
            for label, lineno, value, _src in found.get(name, []):
                print("        suite-class  row %-5d %-14s :%-4d %6.2fs"
                      % (row.get(name, -1), label, lineno, value))
                hit = True
            # A WALK IS AN ARTEFACT OF A PAST TREE; THIS PASS SCANS THE PRESENT
            # ONE. That gap has a direction and it is the flattering one: repair
            # a member and its own red in an old walk stops attributing, so the
            # report degrades from "0 UNATTRIBUTED" to an alarm that reads as
            # "a red appeared from outside the class" on the day a fix landed.
            # Measured, not predicted: 2.775 repaired `reboot` and this pass —
            # which that session owed a re-run of and never spent — turned the
            # walk's row-42 red UNATTRIBUTED, silently, until 2.776 ran it.
            # FIXED is the ruled record of exactly that, so it is what closes
            # the gap; nothing here is derived from a hash or a date.
            if not hit and name in FIXED:
                print("        suite-class  row %-5d REPAIRED SINCE THIS WALK "
                      "— the window this red was measured against is gone from "
                      "the tree (FIXED); the walk still holds the red"
                      % row.get(name, -1))
                hit = True
            for rel in sorted(d_windows):
                if name not in used.get(rel, ()):
                    continue
                for label, lineno, value, _src in d_windows[rel]:
                    print("        driver       %-30s %-14s :%-4d %6.2fs  "
                          "exposure %d suites"
                          % (rel, label, lineno, value, len(used[rel])))
                    hit = True
            if not hit:
                print("        UNATTRIBUTED — no ruled window anywhere on this "
                      "suite's path")
                unattributed.append(name)
        print("\n%d red(s) in this walk: %d with a ruled window on their path, "
              "%d UNATTRIBUTED %s"
              % (len(reds), len(reds) - len(unattributed), len(unattributed),
                 sorted(unattributed) if unattributed else ""))
        walk_drift = bool(unattributed)
        if args.check and walk_drift:
            print("UNATTRIBUTED RED — a red outside the window class is not a "
                  "contention story by default. Rule it before assuming it is.")

    if args.check and (suite_drift or driver_drift or walk_drift):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
