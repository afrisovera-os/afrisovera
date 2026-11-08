#!/usr/bin/env bash
# Phase 2.775 FALSIFIER — `reboot`'s drain(9), both shapes, offline.
#
# WHAT IS BEING TESTED
# --------------------
# Not the kernel. The WAIT that reboot-test.sh:37 and :41 use — a hard 9 s per
# boot phase with NO fall-through — against the replacement: a ceiling with the
# read returning on the guest's own banner.
#
# WHY THIS SUITE AND WHY NOW
# --------------------------
# docs/audit-2.774-walk-reds-vs-census.md §4: of the four reds the live 684-suite
# walk has produced, TWO (`reboot`, `shell-wfi`) sit in no fix queue at all,
# while all three queued conversions (`pi`/`dhcp`/`config`) are PASS at 8/10/17 s.
# `reboot` is the worse of the two promoted: 9 s with no fall-through, against a
# shared driver that allows 90 s for the same boot. Its walk log
# (.rat-logs-2770/rat-reboot.log) is two lines — a cargo line and
# `REBOOT TEST FAILED (shell never came up in boot 1)`.
#
# WHY NO QEMU AND NO CARGO
# ------------------------
# The walk is live and owns this box. A contending guest boot reds rows for no
# kernel reason — the exact defect under audit — so the GUEST is replaced by
# scripts/replay-serial-2775.py, which paces this walk's own row-2 boot capture
# (.rat-logs-2770/mansa-smp.log, 91 lines, banner at line 90) over a real TCP
# serial socket and can perform the reset. Zero boots, zero builds.
#
# THE ARMS. Arms 1 and 5 exist so a rig wired to fail cannot pass this.
#   1  OLD + fast guest         -> PASS   (the old shape is not broken per se)
#   2  OLD + slow guest         -> FAIL with reboot-test.sh's OWN message,
#                                  `shell never came up in boot 1` — the WALK's
#                                  message, on a guest that booted correctly.
#                                  THIS IS THE DEFECT.
#   3  NEW + the SAME slow guest-> PASS, returning inside the ceiling
#   4  NEW + genuinely broken   -> FAIL at the ceiling and STILL with the suite's
#      guest (acknowledges         own `PSCI reset did not restart the machine`:
#      `reboot`, never resets)     the ceiling must not swallow a real fault
#   5  NEW + fast guest         -> PASS in LESS wall clock than arm 1: the refund
#   6  NEW + guest that DIES    -> must NOT spin to the ceiling. recv() on a dead
#      mid-wait                    peer returns b'' forever; a wait that only
#                                  handles socket.timeout would burn the full
#                                  90 s where HEAD's window burned 9. This is the
#                                  fix's own worst failure mode, so it is an arm.
#
# EXIT
#   0  FALSIFY-OK       all six arms reached with the expected verdicts
#   1  FALSIFY-BROKEN   an arm did not do what the fix claims
#   2  cannot run (missing capture / python / client / guest)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

CAP="${CAP:-.rat-logs-2770/mansa-smp.log}"
GUEST="scripts/replay-serial-2775.py"
CLIENT="scripts/reboot-client-2775.py"
[ -f "$CAP" ]    || { echo "NO-RUN: $CAP missing";    exit 2; }
[ -f "$GUEST" ]  || { echo "NO-RUN: $GUEST missing";  exit 2; }
[ -f "$CLIENT" ] || { echo "NO-RUN: $CLIENT missing"; exit 2; }
command -v python >/dev/null 2>&1 || { echo "NO-RUN: no python"; exit 2; }

WORK="${TMPDIR:-/tmp}/rwf2775"
mkdir -p "$WORK"
now_ms() { date +%s%3N; }

# run <mode> <pace_ms> <cap_s> <tag> [NO_RESET] [EOF_AFTER]
# Boots the replay guest on an OS-CHOSEN port (bind 0 + portfile: there is no
# free-port race, and no chance of colliding with the live walk's 4662), runs the
# client in the requested shape, and records verdict + wall clock.
run() {
    local mode="$1" pace="$2" cap="$3" tag="$4" noreset="${5:-}" eofafter="${6:-0}"
    local pf="$WORK/$tag.port" out="$WORK/$tag.out" err="$WORK/$tag.err"
    local t0 gpid port i
    rm -f "$pf"
    REPLAY_LOG="$CAP" REPLAY_PACE_MS="$pace" REPLAY_NO_RESET="$noreset" \
    REPLAY_EOF_AFTER="$eofafter" REPLAY_PORTFILE="$pf" REPLAY_HOLD_S=240 \
        python "$GUEST" -M virt -cpu cortex-a72 -smp 1 -m 256M -display none \
        -serial tcp:127.0.0.1:0,server,nowait -kernel target/mansa-kernel \
        >"$WORK/$tag.guest" 2>&1 &
    gpid=$!
    port=""
    for i in $(seq 1 100); do
        [ -s "$pf" ] && { port="$(tr -d '\r\n ' <"$pf")"; break; }
        sleep 0.1
    done
    if [ -z "$port" ]; then
        kill "$gpid" 2>/dev/null; wait "$gpid" 2>/dev/null
        VERDICT="NO-GUEST"; ELAPSED_MS=0; WAITMSG=""; return 3
    fi
    t0="$(now_ms)"
    MODE="$mode" REBOOT_PORT="$port" REBOOT_WINDOW=9 REBOOT_CEILING="$cap" \
        python "$CLIENT" >"$out" 2>"$err"
    local rc=$?
    ELAPSED_MS=$(( $(now_ms) - t0 ))
    kill "$gpid" 2>/dev/null; wait "$gpid" 2>/dev/null
    VERDICT="$(tr -d '\r' <"$out" | tail -1)"
    WAITMSG="$(tr -d '\r' <"$err" | tail -1)"
    return $rc
}

FAILED=0
note() { printf '  %s\n' "$*"; }
expect() { # <what> <got-rc> <want-rc>
    if [ "$2" -eq "$3" ]; then note "EXPECTED: $1"; else
        note "UNEXPECTED: $1 (rc=$2, wanted $3)"; FAILED=1; fi
}

LINES_IN_CAP="$(wc -l <"$CAP" | tr -d ' ')"
echo "=== 2.775 falsifier: reboot's drain(9) vs a ceiling, offline (capture: $CAP, $LINES_IN_CAP lines) ==="
echo "    fast guest = 5 ms/line (~0.5 s to banner); slow guest = 150 ms/line (~13.6 s to banner)"
echo "    9 s is HEAD's window; the slow guest is a CORRECT guest that misses it."

# ARM 1 -----------------------------------------------------------------------
echo
echo "[arm 1] OLD (drain 9, twice) + FAST guest"
run old 5 9 arm1; rc=$?
note "elapsed ${ELAPSED_MS} ms"
note "verdict: $VERDICT"
expect "arm 1 PASSES — the old shape is not broken per se" "$rc" 0
ARM1_MS=$ELAPSED_MS

# ARM 2 -----------------------------------------------------------------------
echo
echo "[arm 2] OLD (drain 9, twice) + SLOW guest (correct, just slower than the window)"
run old 150 9 arm2; rc=$?
note "elapsed ${ELAPSED_MS} ms"
note "verdict: $VERDICT"
expect "arm 2 FAILS — the window closed on a correct guest" "$rc" 1
if [ "$VERDICT" = "REBOOT TEST FAILED (shell never came up in boot 1)" ]; then
    note "AND it is the WALK's message verbatim (.rat-logs-2770/rat-reboot.log line 2):"
    note "a harness window reported under the kernel's name."
else
    note "UNEXPECTED: arm 2 failed with '$VERDICT', not the walk's message"; FAILED=1
fi

# ARM 3 -----------------------------------------------------------------------
echo
echo "[arm 3] NEW (ceiling 60, read to the banner) + the SAME slow guest"
run new 150 60 arm3; rc=$?
note "elapsed ${ELAPSED_MS} ms"
note "verdict: $VERDICT"
expect "arm 3 PASSES — a slow-but-correct guest is a slow PASS, not a red" "$rc" 0
if [ "$ELAPSED_MS" -lt 60000 ]; then
    note "AND it returned on the guest's OUTPUT, not on the ceiling (${ELAPSED_MS} ms of 60000)"
else
    note "UNEXPECTED: arm 3 burned the ceiling — the ceiling is still a window"; FAILED=1
fi

# ARM 4 -----------------------------------------------------------------------
echo
echo "[arm 4] NEW (ceiling 6) + BROKEN guest: acknowledges 'reboot', never resets"
run new 5 6 arm4 1; rc=$?
note "elapsed ${ELAPSED_MS} ms"
note "wait line: $WAITMSG"
note "verdict: $VERDICT"
expect "arm 4 FAILS — a real fault is still a fail" "$rc" 1
case "$VERDICT" in
    "REBOOT TEST FAILED (AfriSh ready seen 1x; PSCI reset did not restart the machine)")
        note "AND under the SUITE's own name: the ceiling did not swallow the fault" ;;
    *) note "UNEXPECTED: arm 4's message is '$VERDICT', not the suite's own"; FAILED=1 ;;
esac

# ARM 5 -----------------------------------------------------------------------
echo
echo "[arm 5] NEW (ceiling 60) + FAST guest — the refund"
run new 5 60 arm5; rc=$?
note "elapsed ${ELAPSED_MS} ms"
note "verdict: $VERDICT"
expect "arm 5 PASSES" "$rc" 0
if [ "$ELAPSED_MS" -lt "$ARM1_MS" ]; then
    note "AND it cost ${ELAPSED_MS} ms where the fixed windows burned ${ARM1_MS} ms — refund $(( ARM1_MS - ELAPSED_MS )) ms per run"
else
    note "UNEXPECTED: the new shape is not cheaper on a fast host (${ELAPSED_MS} vs ${ARM1_MS} ms)"; FAILED=1
fi

# ARM 6 -----------------------------------------------------------------------
echo
echo "[arm 6] NEW (ceiling 60) + guest that DIES 3 s into boot 1 — the fix's own worst case"
run new 150 60 arm6 "" 3; rc=$?
note "elapsed ${ELAPSED_MS} ms"
note "wait line: $WAITMSG"
note "verdict: $VERDICT"
expect "arm 6 FAILS — a dead guest is still a fail" "$rc" 1
if [ "$ELAPSED_MS" -lt 20000 ]; then
    note "AND it gave up in ${ELAPSED_MS} ms, not at the 60000 ms ceiling: recv()==b'' ends the wait"
else
    note "UNEXPECTED: the wait spun on a dead socket for ${ELAPSED_MS} ms — the ceiling made a dead guest MORE expensive"; FAILED=1
fi

echo
if [ "$FAILED" -eq 0 ]; then
    echo "FALSIFY-OK  (6/6 arms reached; drain(9) is the defect and the ceiling+read repairs it, zero boots spent)"
    exit 0
fi
echo "FALSIFY-BROKEN  (an arm did not do what the fix claims — do NOT apply the fix)"
exit 1
