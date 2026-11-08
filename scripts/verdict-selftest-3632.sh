#!/usr/bin/env bash
# THE VERDICT GRADER'S FALSIFICATION HARNESS (3.632).
#
# scripts/lib/verdict.sh decides what every row of every walk means. This file is
# its second caller (3.184: the expressions are SOURCED, never copied) and its job
# is to reach EVERY branch of classify_verdict offline -- before a walk pays for it
# (Law §9: reach both arms offline before spending the run).
#
# Three kinds of arm:
#   ACCEPT  -- a real banner from the registry must still be honoured
#   REJECT  -- a build artefact / self-test chatter must NOT stand in for a verdict
#   BRANCH  -- classify_verdict must return each of TIMEOUT|PASS|NO-VERDICT|FAIL|DIED
#
# The REJECT arms are not invented: two of them are the VERBATIM lines that the old
# unanchored `PASSED|FAILED|FAIL` matched on the closed, all-green walk 3600
# (.rat-logs-3600/rat-pi.log:3466 and .rat-logs-3600/rat-rxsplit.log:301), where a
# FAIL-family match is by construction a false positive.
#
# Exit 0 and print VERDICT-SELFTEST-GREEN only if every arm holds AND all five
# branches were reached.
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/verdict.sh
. "$here/lib/verdict.sh"

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
ok=0; bad=0
declare -A REACHED=()

note() { printf '  %-6s %-46s %s\n' "$1" "$2" "$3"; }
pass() { ok=$((ok + 1));  note ok   "$1" "$2"; }
fail() { bad=$((bad + 1)); note FAIL "$1" "$2"; }

# write_log <name> <line...>  -> echoes the path
write_log() { local n="$1"; shift; local p="$tmp/$n.log"; printf '%s\n' "$@" > "$p"; printf '%s' "$p"; }

# expect_class <label> <want> <rc> <logpath>
expect_class() {
    local label="$1" want="$2" rc="$3" log="$4" got
    got="$(classify_verdict "$rc" "$log")"
    REACHED[$got]=1
    if [ "$got" = "$want" ]; then pass "$label" "-> $got"; else fail "$label" "want=$want got=$got"; fi
}

# expect_re <label> <accept|reject> <pass|fail> <line>
expect_re() {
    local label="$1" want="$2" which="$3" line="$4" p hit
    p="$(write_log "re$((ok + bad))" "$line")"
    if [ "$which" = pass ]; then verdict_saw_pass "$p"; else verdict_saw_fail "$p"; fi
    hit=$?
    if { [ "$want" = accept ] && [ $hit -eq 0 ]; } || { [ "$want" = reject ] && [ $hit -ne 0 ]; }
    then pass "$label" "$which-re $want"; else fail "$label" "$which-re wanted $want, got hit=$hit"; fi
}

echo "== verdict-selftest 3632 =="
echo "-- ACCEPT: the banners the registry actually prints (688 TEST / 1 SUITE, measured on walk 3600)"
expect_re "banner: <NAME> TEST PASSED"       accept pass 'ALNUM TEST PASSED  (a->1, z->26)'
expect_re "banner: REGRESSION SUITE PASSED"  accept pass '== REGRESSION SUITE PASSED =='
expect_re "banner: <NAME> TEST FAILED"       accept fail 'ALNUM TEST FAILED  (expected 26, got 25)'
expect_re "banner: REGRESSION FAILED: <why>" accept fail 'REGRESSION FAILED: kernel did not boot'
expect_re "banner: line-initial TEST PASSED" accept pass 'TEST PASSED'

echo "-- REJECT: artefacts the OLD unanchored rule matched (the measured defect)"
expect_re "walk3600 rat-pi.log:3466 (rustc)"   reject fail 'warning: constant `S_FAILED` is never used'
expect_re "walk3600 rat-pi.log:3469 (rustc)"   reject fail '51 | const S_FAILED: u32 = 128;'
expect_re "walk3600 rat-rxsplit.log:301"       reject fail '  | GRADER SELFTEST: 14 ok / 0 FAIL'
expect_re "walk3600 rat-rxsplit.log:15"        reject fail '  grader selftest: 12 ok / 0 FAIL'
expect_re "hyphenated SELFTEST-FAILED"         reject fail 'SELFTEST-FAILED: arm 3'
expect_re "hyphenated PROBE-PREMISE-FAILED"    reject fail 'PROBE-PREMISE-FAILED'
expect_re "prose: 0 tests FAILED-free"         reject fail 'nothing FAILED_HERE today'

echo "-- REJECT: the word-boundary arm (a GUARD, not a measured defect: 0 of 689 logs and"
echo "           0 of 689 suite sources print one of these today)"
expect_re "SELFTEST PASSED is not a verdict"   reject pass 'GRADER SELFTEST PASSED'
expect_re "RETEST PASSED is not a verdict"     reject pass 'RETEST PASSED'
expect_re "SELFTEST FAILED is not a verdict"   reject fail 'GRADER SELFTEST FAILED'

echo "-- REJECT: a PASS branch that must not accept a FAILED banner (a fold here would flatter)"
expect_re "TEST FAILED never reads as pass"    reject pass 'ALNUM TEST FAILED'

echo "-- BRANCH: all five outcomes of classify_verdict"
expect_class "rc=124 -> TIMEOUT"    TIMEOUT    124 "$(write_log t124 'ALNUM TEST PASSED')"
expect_class "rc=137 -> TIMEOUT"    TIMEOUT    137 "$(write_log t137 'anything')"
expect_class "rc=0 + banner"        PASS         0 "$(write_log p0   'ALNUM TEST PASSED')"
expect_class "rc=0, no banner"      NO-VERDICT   0 "$(write_log nv   'built ok' 'GRADER SELFTEST PASSED')"
expect_class "rc=1 + FAILED banner" FAIL         1 "$(write_log f1   'ALNUM TEST FAILED (2 of 9)')"
expect_class "rc=1 + PASSED banner" FAIL         1 "$(write_log f2   'ALNUM TEST PASSED' 'then the shell died')"
expect_class "rc=1, artefacts only" DIED         1 "$(write_log d1   'warning: constant `S_FAILED` is never used' '  | GRADER SELFTEST: 14 ok / 0 FAIL')"
expect_class "rc=1, missing log"    DIED         1 "$tmp/does-not-exist.log"

echo "-- CONTROL: the OLD rule on the SAME 'died with artefacts only' log"
old_hit=1; grep -qE 'PASSED|FAILED|FAIL' "$tmp/d1.log" && old_hit=0
if [ $old_hit -eq 0 ]; then pass "old rule calls it FAIL (the defect)" "reproduced"
else fail "old rule calls it FAIL (the defect)" "did NOT reproduce -- premise gone"; fi

echo "-- INSTRUMENT: the runner must hold NO second copy of the rule (3.184) and NO"
echo "               control bytes (3.633: 0x08 backspaces were written for \b escapes)"
RUNNER="$here/run-all-tests.sh"
if grep -q "lib/verdict.sh" "$RUNNER"; then pass "runner sources lib/verdict.sh" ""
else fail "runner sources lib/verdict.sh" "no reference found"; fi

n_copy=$(grep -cE "^[[:space:]]*VERDICT_(PASS|FAIL|ANY)_RE=" "$RUNNER")
if [ "$n_copy" -eq 0 ]; then pass "runner defines no verdict regex of its own" "0 definitions"
else fail "runner defines no verdict regex of its own" "$n_copy inline definition(s) -- a second copy"; fi

# 3.634: control bytes are counted with `tr`, NOT `grep -P`.
# The original detector was `LC_ALL=C grep -cP "[\x00-\x08...]" FILE || true`. On this host
# that grep ABORTS -- "grep: -P supports only unibyte and UTF-8 locales" -- so the command
# substitution captured the EMPTY STRING, `|| true` swallowed the failure, and `${n_ctl:-0}`
# turned the crash into 0, i.e. into "clean". Verified 3.634 on a file carrying one real
# 0x08: the arm printed `ok ... clean`. The guard against 3.633's exact byte COULD NOT FAIL.
# A guard that cannot fail is worse than no guard -- it reports assurance it never checked,
# and it fails FLATTERING, which is the direction this project keeps getting caught by.
#
# `tr -d` is POSIX, needs no PCRE and no locale, and cannot half-succeed: strip every control
# byte and compare byte counts. And the detector now ships with a POSITIVE CONTROL that makes
# it fail on demand -- 3.634's rule: a guard ships with a case that makes it fire, or it is
# not known to be a guard.
ctl_bytes() {   # ctl_bytes <file> -> count of control bytes (NUL..BS, VT, FF, SO..US)
    local f="$1" tot stripped
    [ -f "$f" ] || { echo MISSING; return 0; }
    tot=$(wc -c < "$f")
    stripped=$(LC_ALL=C tr -d '\000-\010\013\014\016-\037' < "$f" | wc -c)
    echo $(( tot - stripped ))
}

# POSITIVE CONTROL FIRST: a file carrying one real 0x08 must be DETECTED. If this arm does
# not fire, every "clean" verdict below is vacuous and means nothing.
printf 'VERDICT_PASS_RE=(^|\b)(TEST|SUITE) PASSED\n' > "$tmp/poisoned.sh"
n_poison=$(ctl_bytes "$tmp/poisoned.sh")
if [ "$n_poison" != MISSING ] && [ "$n_poison" -ge 1 ]
then pass "detector FIRES on a real 0x08 (positive control)" "$n_poison byte(s)"
else fail "detector FIRES on a real 0x08 (positive control)" "found $n_poison -- guard is VACUOUS"; fi

n_ctl=$(ctl_bytes "$RUNNER")
if [ "$n_ctl" = "0" ]; then pass "runner carries no control bytes" "clean"
else fail "runner carries no control bytes" "$n_ctl byte(s) -- see 3.633"; fi

n_ctl_lib=$(ctl_bytes "$here/lib/verdict.sh")
if [ "$n_ctl_lib" = "0" ]; then pass "library carries no control bytes" "clean"
else fail "library carries no control bytes" "$n_ctl_lib byte(s)"; fi

n_ctl_self=$(ctl_bytes "${BASH_SOURCE[0]}")
if [ "$n_ctl_self" = "0" ]; then pass "selftest itself carries no control bytes" "clean"
else fail "selftest itself carries no control bytes" "$n_ctl_self byte(s)"; fi

echo "-- COVERAGE: every branch label reached"
for b in TIMEOUT PASS NO-VERDICT FAIL DIED; do
    if [ -n "${REACHED[$b]:-}" ]; then pass "branch reached: $b" ""; else fail "branch reached: $b" "never returned"; fi
done

echo
echo "verdict-selftest: $ok ok / $bad FAIL"
if [ "$bad" -eq 0 ]; then echo "VERDICT-SELFTEST-GREEN"; exit 0; fi
echo "VERDICT-SELFTEST-RED"; exit 1
