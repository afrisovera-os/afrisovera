#!/usr/bin/env bash
# TCP INUSE BOOT ARM check (suite, milestone 3.668): when something ELSE already
# holds 127.0.0.1:80 before tcp-test.sh boots, the suite must take its documented
# INUSE branch, grade the GUEST (not the port owner), and leave the foreign holder
# untouched -- never silently pass by accident, never kill a process it never
# owned, never report a broken listener as if it were a busy one.
#
# THIS SUITE DOES NOT REIMPLEMENT THE MEASUREMENT. It CALLS
# scripts/inuse-boot-probe-2859.sh, unedited, and grades what that probe reports --
# the same shape as suite tcp-ooo/tcp-dest, for the same two reasons (2.765 rule
# 5, reinforced at 2.766/2.767):
#   * the acceptance criterion must be a script that existed BEFORE this suite was
#     written. This one walked the whole milestone from the outside and was
#     falsified on real back-to-back QEMU boots at 3.642 (commit 3148be1): exit 0
#     (INUSE-BOOT-OK) with a genuine foreign holder, and its own numbered checks
#     (1: branch taken, 3: losing listener accepted nothing, 4: the GUEST reached
#     the foreign socket through slirp) are what make the branch DIFFERENTIAL
#     rather than merely reachable -- 2.953 had reached the branch without a
#     no-holder control and left arms 1/3 ungraded as "differential OR the
#     listener is simply broken and reads the same either way". A checker
#     authored afterwards is authored by someone who already knows the branch is
#     healthy;
#   * when a falsified probe already exists in the tree, CALLING it is the only
#     permitted form -- a re-typed copy of its logic is a new artefact that has
#     never been falsified.
# So everything below the probe call is grading, never measuring. If this suite
# and the probe ever disagree about the kernel, the probe is right.
#
# 3.642 ALSO built scripts/inuse-control-3639.sh, inuse-selftest-3639.sh and
# inuse-orphandiff-3641.sh to prove the branch is differential rather than the
# suite's listener being dead either way -- that proof is DONE (16/16, 14/14,
# 9/9, all rc=0 on fresh boots, re-derived from a session that did not write
# them) and is NOT re-run here. Re-running it on every regression walk would add
# two more QEMU boots to every suite invocation to re-litigate a question this
# suite does not ask: THIS suite's only job is "does tcp-test.sh still behave
# correctly under a foreign holder, on THIS kernel, right now" -- the same
# narrower job tcp-dest/tcp-ooo do for their own probes.
#
# WHAT IS GRADED, and why each line is here rather than the obvious alternative:
#   1. the probe's EXIT CODE, captured off a redirect and NOT a pipe. `probe | tee`
#      hands back tee's 0 no matter what the probe decided -- the 2.765 hole where
#      `gate | tail -25` returned tail's 0 off a RED gate.
#   2. the probe's PRINTED verdict word (INUSE-BOOT-OK), independently of the exit
#      code. A lost, masked or inherited exit code cannot green this suite alone.
#   3. the probe's own "N passed, M failed" summary line, cross-checked against
#      its 8 documented checks -- so a probe that silently drops a check (prints
#      fewer than 8) cannot pass by omission.
#   4. FIVE of the probe's eight checks named individually, not just summed,
#      because a miscount in the summary line must not be able to hide WHICH
#      property broke: (1) the branch was actually taken (HOST-LISTENER=foreign),
#      (3) the suite's own losing listener accepted nothing (it never owned the
#      socket), (4) the guest's SYN reached the FOREIGN holder through slirp (the
#      branch is differential, not merely taken), (7) the suite's own exit code
#      is 0 (it graded the guest, not the port owner), (8) tcp-test.sh itself
#      printed its normal TCP TEST PASSED verdict under the held port.
#
# WHAT IS DELIBERATELY NOT GRADED: checks 2, 5 and 6 of the probe (the suite's
# own listener log line, the foreign holder staying alive, the foreign holder
# still serving afterward). All three passed on every observed run and are
# already covered by the probe's own summary-line count (item 3 above) -- naming
# every single one here would just be re-typing the probe's own arithmetic
# without adding a new failure mode this suite can catch that the summary count
# does not already catch. Same discipline as tcp-dest's "the peer answered" and
# tcp-ooo's "the 150ms recovery": grade the property the milestone is ABOUT, not
# every printed line for its own sake.
#
# Runs from the repo root (the probe resolves scripts/... and target/...
# relatively). Needs python3 and qemu-system-aarch64 on PATH; the runner puts
# both there. Binds 127.0.0.1:80 for its foreign holder -- if something else on
# the HOST already owns :80, the probe's own precondition check catches it and
# exits 10 (INUSE-NOT-REACHED) rather than silently measuring the wrong thing.
# Takes roughly a minute -- one QEMU boot, one held port.
set -uo pipefail

PROBE="scripts/inuse-boot-probe-2859.sh"
T="${TMPDIR:-/tmp}"
OUT="$T/tcp-inuse.out"
FAIL=0
fail() { echo "  FAIL: $*"; FAIL=1; }

if [ ! -f "$PROBE" ]; then
    echo "TCP-INUSE TEST FAILED  (the probe this suite grades is missing: $PROBE)"
    exit 1
fi

# --------------------------------------------------------------------------
# the grader: takes the probe's OUTPUT and EXIT CODE, sets FAIL. No QEMU here,
# nothing kernel-specific -- which is what lets every arm be reached offline
# before a run is spent on it (Law section 9, 2.766).
# --------------------------------------------------------------------------
grade() {                     # grade <probe-output-file> <probe-exit-code>
    local F="$1" RC="$2"
    local VERDICT SUMLINE NPASS NFAIL
    FAIL=0

    # 1. the exit code
    case "$RC" in
        0)  echo "  probe exit 0 -- INUSE-BOOT-OK" ;;
        10) fail "probe exit 10 -- INUSE-NOT-REACHED: the suite never took the INUSE branch (foreign holder never bound, or HOST-LISTENER=foreign never printed)" ;;
        20) fail "probe exit 20 -- INUSE-PARTIAL: the branch was reached but at least one of the probe's other checks failed" ;;
        *)  fail "probe exit $RC -- not one of the three verdicts it documents (0/10/20)" ;;
    esac

    # 2. the printed verdict, derived independently of the exit code
    VERDICT="$(grep -oE 'INUSE-(BOOT-OK|NOT-REACHED|PARTIAL)' "$F" | tail -1)"
    [ "$VERDICT" = INUSE-BOOT-OK ] ||
        fail "the probe PRINTED '${VERDICT:-<nothing>}', not INUSE-BOOT-OK -- the exit code is not the only reader"

    # 3. the probe's own summary line, cross-checked against its documented 8 checks
    SUMLINE="$(grep -oE -- '-- [0-9]+ passed, [0-9]+ failed --' "$F" | tail -1)"
    if [ -z "$SUMLINE" ]; then
        fail "the probe never printed its own 'N passed, M failed' summary line -- nothing to cross-check the exit code against"
    else
        NPASS="$(printf '%s\n' "$SUMLINE" | grep -oE '^-- [0-9]+' | grep -oE '[0-9]+')"
        NFAIL="$(printf '%s\n' "$SUMLINE" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+')"
        [ "$NFAIL" = "0" ] ||
            fail "the probe's own summary says $NFAIL of its checks failed ($SUMLINE)"
        [ "$NPASS" = "8" ] ||
            fail "the probe's own summary says only $NPASS of its 8 documented checks passed ($SUMLINE)"
    fi

    # 4. five load-bearing checks named individually -- a summary miscount must
    #    not be able to hide WHICH property broke
    grep -q '^  ok   1 suite took the INUSE branch' "$F" ||
        fail "check 1 did not pass -- the suite never took the INUSE branch (HOST-LISTENER=foreign)"
    grep -q '^  ok   3 the suite' "$F" ||
        fail "check 3 did not pass -- the suite's losing listener accepted a connection it never should have owned"
    grep -q '^  ok   4 the GUEST reached the FOREIGN socket' "$F" ||
        fail "check 4 did not pass -- the guest's SYN never reached the foreign holder through slirp -- the branch is taken but not proven differential"
    grep -q '^  ok   7 suite rc=0' "$F" ||
        fail "check 7 did not pass -- tcp-test.sh's own exit code was not 0 -- it graded the port owner, not the guest"
    grep -q '^  ok   8 verdict: TCP TEST PASSED' "$F" ||
        fail "check 8 did not pass -- tcp-test.sh did not print its normal TCP TEST PASSED verdict under the held port"

    return "$FAIL"
}

# --------------------------------------------------------------------------
# the grader's own selftest: both arms of every predicate above, offline, with
# no QEMU spent. Law section 9 (2.766): a check whose failing arm has never been
# reached is a check nobody has tested. It runs on every real invocation too --
# it costs nothing.
#
# The fixture is the REAL captured output of scripts/inuse-boot-probe-2859.sh run
# standalone this session (rc=0, INUSE-BOOT-OK, 8/8 checks passed -- the same
# shape 3.642 measured on fresh boots). Mutants are sed edits of it.
# --------------------------------------------------------------------------
green_fixture() {
    cat <<'EOF'
foreign holder: READY 127.0.0.1:80 (pid 21352)
== running scripts/tcp-test.sh (unedited) against the held port ==
suite rc=0
  | HOST-LISTENER=foreign (something already listens on 127.0.0.1:80)
  |   host listener accepted: not measured this run - a FOREIGN listener owns 127.0.0.1:80 and keeps its own log
  | TCP TEST PASSED  (full 3-way handshake: SYN -> SYN/ACK -> ACK, checksums OK)
== grading ==
  ok   1 suite took the INUSE branch: HOST-LISTENER=foreign (something already listens on 127.0.0.1:80)
  ok   2 the suite's own listener log says INUSE: INUSE 127.0.0.1:80 already has a listener
  ok   3 the suite's losing listener accepted nothing (it never owned the socket)
  ok   4 the GUEST reached the FOREIGN socket: 1 ACCEPTED - ACCEPTED ('127.0.0.1', 60600)
  ok   5 foreign holder still alive after the suite (LPID="" honoured: no stranger killed)
  ok   6 foreign holder still SERVES after the suite
  ok   7 suite rc=0 (it graded the GUEST, not the port owner)
  ok   8 verdict: TCP TEST PASSED  (full 3-way handshake: SYN -> SYN/ACK -> ACK, checksums OK)
-- 8 passed, 0 failed --
INUSE-BOOT-OK
EOF
}

selftest() {
    local G="$T/tcp-inuse.fixture" M="$T/tcp-inuse.mutant" ok=0 bad=0

    green_fixture > "$G"

    check() {                 # check <name> <want pass|fail> <file> <rc>
        local name="$1" want="$2" f="$3" rc="$4" got
        grade "$f" "$rc" > /dev/null 2>&1
        if [ "$FAIL" -eq 0 ]; then got=pass; else got=fail; fi
        if [ "$got" = "$want" ]; then
            ok=$((ok + 1)); printf '  PASS  %-52s -> %s\n' "$name" "$got"
        else
            bad=$((bad + 1)); printf '  FAIL  %-52s -> %s (wanted %s)\n' "$name" "$got" "$want"
        fi
    }

    # the accepting arm
    check "the real green run grades clean" pass "$G" 0

    # every rejecting arm, one mutation each
    check "a non-zero probe exit is caught" fail "$G" 20
    sed 's/^INUSE-BOOT-OK/INUSE-PARTIAL/' "$G" > "$M"
    check "printed verdict disagreeing with exit 0" fail "$M" 0
    sed 's/-- 8 passed, 0 failed --/-- 7 passed, 1 failed --/' "$G" > "$M"
    check "the probe's own summary reports a failure" fail "$M" 0
    sed 's/-- 8 passed, 0 failed --/-- 7 passed, 0 failed --/' "$G" > "$M"
    check "the probe's own summary drops a check silently" fail "$M" 0
    sed "s/^  ok   1 suite took the INUSE branch.*/  FAIL 1 suite did NOT take the INUSE branch/" "$G" > "$M"
    check "check 1 (branch taken) failing is caught" fail "$M" 0
    sed "s/^  ok   3 the suite.*/  FAIL 3 the suite's losing listener accepted a connection/" "$G" > "$M"
    check "check 3 (losing listener silent) failing is caught" fail "$M" 0
    sed "s/^  ok   4 the GUEST reached the FOREIGN socket.*/  FAIL 4 foreign holder accepted 0 connections/" "$G" > "$M"
    check "check 4 (differential, not just taken) failing is caught" fail "$M" 0
    sed "s/^  ok   7 suite rc=0.*/  FAIL 7 suite rc=1, expected 0/" "$G" > "$M"
    check "check 7 (suite's own exit code) failing is caught" fail "$M" 0
    sed "s/^  ok   8 verdict: TCP TEST PASSED.*/  FAIL 8 no TCP TEST PASSED line/" "$G" > "$M"
    check "check 8 (normal verdict under the held port) failing is caught" fail "$M" 0

    if [ "$bad" -eq 0 ]; then
        echo "GRADER SELFTEST PASS ($ok/$((ok + bad)), every rejecting arm reached)"
        return 0
    fi
    echo "GRADER SELFTEST FAIL ($ok/$((ok + bad)))"
    return 1
}

# --------------------------------------------------------------------------
if [ "${1:-}" = "--selftest" ]; then
    selftest
    exit $?
fi

STOUT="$T/tcp-inuse.selftest.out"
bash "$0" --selftest > "$STOUT" 2>&1
if ! grep -q 'GRADER SELFTEST PASS' "$STOUT"; then
    sed 's/^/  /' "$STOUT"
    echo "TCP-INUSE TEST FAILED  (the grader's own arithmetic did not pass its selftest -- nothing was measured)"
    exit 1
fi
sed 's/^/  /' "$STOUT"

echo "  running $PROBE unedited (0=INUSE-BOOT-OK, 10=INUSE-NOT-REACHED, 20=INUSE-PARTIAL)..."
bash "$PROBE" > "$OUT" 2>&1
RC=$?                        # captured off a redirect: no pipe stands between the
                             # probe's verdict and this variable
echo "  --- the probe (exit $RC) ---"
sed 's/^/  /' "$OUT"
echo "  --- grading it ---"

grade "$OUT" "$RC"

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-INUSE TEST PASSED  (a FOREIGN holder on 127.0.0.1:80 forces tcp-test.sh down its documented INUSE branch, the guest's SYN reaches the foreign holder through slirp -- not the suite's own dead listener -- and the suite still grades the guest and prints its normal TCP TEST PASSED verdict, leaving the foreign holder untouched)"
    exit 0
fi
echo "TCP-INUSE TEST FAILED"
exit 1
