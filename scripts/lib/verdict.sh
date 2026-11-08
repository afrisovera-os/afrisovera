# shellcheck shell=bash
# THE VERDICT GRADER (3.632) -- ONE definition, TWO callers.
#
# WHY THIS FILE EXISTS. run-all-tests.sh decided a suite's fate with two
# UNANCHORED substring greps:
#
#     if grep -q "PASSED" "$log"; then st=PASS; else st=NO-VERDICT; fi   # :633
#     elif grep -qE "PASSED|FAILED|FAIL" "$log"; then st=FAIL            # :634
#
# A verdict is a BANNER a suite prints about itself. "PASSED" / "FAILED" /
# "FAIL" are merely SUBSTRINGS, and a QEMU-booting suite's log carries a whole
# cargo build, so the substrings appear for reasons that have nothing to do with
# any verdict. Measured on the CLOSED, all-green walk 3600 (689 logs, 689 PASS,
# so a FAIL-family match is by construction a FALSE POSITIVE), the old FAIL
# branch matched TWO logs on non-verdict text:
#
#   .rat-logs-3600/rat-pi.log:3466   warning: constant `S_FAILED` is never used
#   .rat-logs-3600/rat-pi.log:3469   51 | const S_FAILED: u32 = 128;
#       -- a Rust compiler warning (found by 3.616 over the 186-row prefix)
#   .rat-logs-3600/rat-rxsplit.log:15    grader selftest: 12 ok / 0 FAIL
#   .rat-logs-3600/rat-rxsplit.log:301   | GRADER SELFTEST: 14 ok / 0 FAIL
#       -- a report of ZERO failures read as evidence of failure; registry
#          position 689/689, so it sat OUTSIDE 3.616's prefix and is new here
#
# Both are latent, not live: at rc == 0 the FAIL branch is never consulted. But
# had either suite exited non-zero for ANY reason, the runner would have called
# it FAIL ("ran, judged itself, lost") when the truth is DIED ("died before
# reaching its own verdict"). That is 2.766's fragment-matching hazard living in
# the instrument that grades every walk.
#
# THE ANCHOR is the delimiter the banners themselves use, not a longer substring
# (Law §9). Surveyed over all 689 registered suites: 688 print "<NAME> TEST
# PASSED" and 682 print "<NAME> TEST FAILED"; smoke -- the one exception -- prints
# "REGRESSION SUITE PASSED" and "REGRESSION FAILED: <why>". So the verdict word is
# always immediately preceded by TEST, SUITE or REGRESSION, and that word must
# itself START a word: `SELFTEST-FAILED`, `RIG-FAILED`, `PROBE-PREMISE-FAILED`,
# `S_FAILED` and `0 FAIL` all fail the anchor, while every real banner passes it.
#
# The bare `FAIL` alternative is DROPPED outright: no suite in the registry ends
# its verdict at `FAIL`, and it is the alternative that matched rxsplit's
# zero-count line.
#
# FAILURE DIRECTION, stated not glossed: this is CONSERVATIVE in both branches. A
# future suite whose banner does not fit the anchor is graded NO-VERDICT (not
# PASS) or DIED (not FAIL) -- it makes the registry look worse, never greener, so
# it can only raise a question, never hide one.
#
# Callers: scripts/run-all-tests.sh (the grading loop) and
# scripts/verdict-selftest-3632.sh (the falsification harness). Neither COPIES
# these expressions -- 3.184: a second copy is a second thing to be wrong.

# A verdict banner: <delimiter word> <verdict word>, both on word boundaries.
VERDICT_PASS_RE='(^|[^A-Za-z0-9_])(TEST|SUITE) PASSED([^A-Za-z0-9_]|$)'
VERDICT_FAIL_RE='(^|[^A-Za-z0-9_])(TEST|SUITE|REGRESSION) FAILED([^A-Za-z0-9_]|$)'

verdict_saw_pass() { [ -f "$1" ] && grep -qE "$VERDICT_PASS_RE" "$1"; }
verdict_saw_fail() { [ -f "$1" ] && grep -qE "$VERDICT_FAIL_RE" "$1"; }

# classify_verdict <rc> <logfile> -> prints one of TIMEOUT|PASS|NO-VERDICT|FAIL|DIED
#
# The rc ladder is unchanged from run-all-tests.sh:629-638; only the two greps it
# consults are anchored.
classify_verdict() {
    local rc="$1" log="$2"
    if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
        echo TIMEOUT                        # hung: killed, recorded, run continues
    elif [ "$rc" -eq 0 ]; then
        # exit 0 is not a verdict -- a suite that never printed one never checked.
        if verdict_saw_pass "$log"; then echo PASS; else echo NO-VERDICT; fi
    elif verdict_saw_pass "$log" || verdict_saw_fail "$log"; then
        echo FAIL                           # ran, judged itself, lost
    else
        echo DIED                           # died before reaching its own verdict
    fi
}
