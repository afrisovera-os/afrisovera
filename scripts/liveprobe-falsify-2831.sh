#!/usr/bin/env bash
# liveprobe-falsify-2831.sh -- the falsifier for the (a2) repair named in 2.830.
#
# 2.830 measured a two-cell table against the LIVE walk: the shipped liveness
# expression at scripts/arm-holders.sh:91 reads ALIVE with a clean env and DEAD
# with MSYS_NO_PATHCONV exported, on the SAME live pid. That is the family's
# flattering direction (a running walk reads as exited), and the site cannot
# notice because its stderr goes to /dev/null.
#
# This script is the falsifier that repair must clear. It is SELF-CONTAINED: it
# spawns its own live process and uses an impossible pid for the dead arm, so it
# never probes -- and never depends on -- the walk. Run it before AND after the
# arm-holders.sh:91 edit; before the edit cell 2 disagreed with the table
# (LIVEPROBE-DEFECT 1/6, exit 10 -- .rat-2833-before.out), after the 2.833 edit
# all six agree (exit 0). NOTE: from 2.833 on, the drift guard below REFUSES
# (exit 32) on a tree whose arm-holders.sh has not been repaired, so the
# "before" arm is reproducible only by reverting the site.
#
# 2.830 second finding, honoured here: the handle's WRITER row carries BOTH pids
# (pid= is the MSYS pid, winpid= is the Windows one). tasklist only knows winpid.
# This script reads /proc/<msyspid>/winpid rather than naming a pid by reflex.
set -u

DEAD_PID=999999          # no Windows pid is ever this; the honest-DEAD arm
FAIL=0

# --- the two forms under test -------------------------------------------------

# SHIPPED: a behaviour mirror of the liveness probe in arm-holders.sh.
#
# BEFORE the 2.833 repair this read `tasklist //FI "PID eq $1" 2>/dev/null |
# grep -q "$1"` -- and cell 2 caught it: with MSYS_NO_PATHCONV exported the
# `//FI` reaches tasklist literally, the error goes to /dev/null, and the empty
# stdout said DEAD about a process this script had just spawned. Evidence of
# that run is kept at .rat-2833-before.out (LIVEPROBE-DEFECT 1/6, exit 10).
#
# AFTER the repair the site sets MSYS_NO_PATHCONV for its own command, so the
# caller's env cannot change the answer, and captures stderr so an errored probe
# is UNKNOWN rather than DEAD. This mirror is a COPY, tied to the real site only
# by the drift guard below -- it is not the site executing.
probe_shipped() {
    local err out
    err="$(mktemp)"
    out="$(MSYS_NO_PATHCONV=1 tasklist /FI "PID eq $1" 2>"$err")"
    if [ -s "$err" ]; then
        rm -f "$err"
        echo UNKNOWN
        return
    fi
    rm -f "$err"
    case "$out" in
        *"$1"*) echo ALIVE ;;
        *)      echo DEAD ;;
    esac
}

# DRIFT GUARD: the mirror above claims to describe arm-holders.sh. If that site
# does not carry the repaired command form, this script is grading a fiction --
# refuse rather than print a green it has not earned.
SITE="$(dirname "$0")/arm-holders.sh"
if ! grep -q 'MSYS_NO_PATHCONV=1 tasklist /FI "PID eq \${WINPID}"' "$SITE" 2>/dev/null; then
    echo "FALSIFY-REFUSED (32): ${SITE} does not carry the repaired probe form; the mirror would be a fiction"
    exit 32
fi
# ...and no LIVE `//FI` probe may survive there. Anchored to skip comment lines:
# the repair's own note quotes the old form verbatim, and the first draft of
# this guard fired on that quote (exit 32) -- a comment is not a probe.
if grep -q '^[^#]*tasklist //FI' "$SITE" 2>/dev/null; then
    echo "FALSIFY-REFUSED (32): ${SITE} still carries a \`tasklist //FI\` probe"
    exit 32
fi

# REPAIRED: same command, same argument. The ONLY change is that stderr is
# captured instead of discarded, and a probe that errored is UNKNOWN -- never
# DEAD. 2.829 established that stderr is the only separator between "switch
# eaten" and "process gone"; 2.828's 2>&1 rule had not reached this line.
probe_repaired() {
    local err out
    err="$(mktemp)"
    out="$(tasklist //FI "PID eq $1" 2>"$err")"
    if [ -s "$err" ]; then
        rm -f "$err"
        echo UNKNOWN
        return
    fi
    rm -f "$err"
    case "$out" in
        *"$1"*) echo ALIVE ;;
        *)      echo DEAD ;;
    esac
}

cell() { # cell <n> <form> <env> <pid> <expected>
    local n="$1" form="$2" envv="$3" pid="$4" want="$5" got
    if [ "$envv" = "nopathconv" ]; then
        got="$(MSYS_NO_PATHCONV=1 "$form" "$pid")"
    else
        got="$("$form" "$pid")"
    fi
    if [ "$got" = "$want" ]; then
        printf '  cell %s  %-14s %-11s pid=%-7s -> %-7s (want %s)  ok\n' \
               "$n" "$form" "$envv" "$pid" "$got" "$want"
    else
        printf '  cell %s  %-14s %-11s pid=%-7s -> %-7s (want %s)  MISMATCH\n' \
               "$n" "$form" "$envv" "$pid" "$got" "$want"
        FAIL=$((FAIL + 1))
    fi
}

# --- a live pid this script owns ---------------------------------------------

tail -f /dev/null &
MSYS_PID=$!
trap 'kill "$MSYS_PID" 2>/dev/null' EXIT
LIVE_PID="$(cat "/proc/${MSYS_PID}/winpid" 2>/dev/null || true)"
[ -n "${LIVE_PID:-}" ] || { echo "FALSIFY-REFUSED: no /proc/${MSYS_PID}/winpid"; exit 30; }

# Ground truth by the OTHER permitted form, before grading anything. If this
# does not see the process, the fixture is broken and no cell below means
# anything -- refuse rather than grade.
if ! MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${LIVE_PID}" 2>&1 | grep -q "${LIVE_PID}"; then
    echo "FALSIFY-REFUSED: ground truth does not see live winpid=${LIVE_PID}"
    exit 31
fi
echo "fixture: msyspid=${MSYS_PID} winpid=${LIVE_PID} confirmed live by ground truth"
echo

echo "SHIPPED form (arm-holders.sh:91) -- expected to FAIL cell 2 before the repair:"
cell 1 probe_shipped  clean      "$LIVE_PID" ALIVE
cell 2 probe_shipped  nopathconv "$LIVE_PID" ALIVE
cell 3 probe_shipped  clean      "$DEAD_PID" DEAD
echo
echo "REPAIRED form -- must never say DEAD about a probe that errored:"
cell 4 probe_repaired clean      "$LIVE_PID" ALIVE
cell 5 probe_repaired nopathconv "$LIVE_PID" UNKNOWN
cell 6 probe_repaired clean      "$DEAD_PID" DEAD
echo

if [ "$FAIL" -eq 0 ]; then
    echo "LIVEPROBE-CLEAR 6/6 -- both forms agree with the table"
    exit 0
fi
echo "LIVEPROBE-DEFECT ${FAIL}/6 cells mismatched"
exit 10
