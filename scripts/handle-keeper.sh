#!/usr/bin/env bash
# handle-keeper.sh -- keep ${RESULTS}.handle's PROGRESS row in step with the
# results file, for as long as the walk is running.
#
# WHY THIS EXISTS (2.855, the gap 2.854 measured on a FIXTURE but not on the
# live walk): stamp-handle.sh:112's `--rows` header says it is "safe to call
# from a launch loop after every row" -- but `grep -rl stamp-handle` finds only
# preflights, selftests and the rehearsal. NOTHING CALLS IT WHILE A WALK RUNS.
# So on the live 2.835 walk the handle sat at rows=255 (last hand-stamp, 11:17Z)
# while the file held 678 (12:20Z), and handle-check.sh answered HANDLE-ROWS (7)
# -- which close-out maps to a refusal. 2.854 made the FIXTURE look in-flight by
# appending a fresh PROGRESS row by hand; this makes the REAL walk actually be
# what the fixture pretends to be.
#
# It derives NOTHING itself: every stamp is stamp-handle.sh --rows, which re-uses
# the WRITER row already on disk verbatim. This file only decides WHEN to call it.
#
# It STOPS rather than lie:
#   - the WRITER is dead but the handle is still open (65) -> stop, rc 65.
#   - handle closed by --exit (ROWS-REFUSED 64) -> the walk ended; stop, rc 0.
#   - handle gone / names another file (61/62/63) -> stop, rc = that code.
#   - MAX_SECS elapsed -> stop, rc 0 (a keeper must not outlive its usefulness).
#
# THE WRITER-LIVENESS ARM (2.857, gap (b) of 2.856, measured LIVE not reasoned):
# the keeper's only stop condition used to be the HANDLE closing, not the WRITER
# dying. On the real 2.835 walk the writer exited 12:24:51Z and for 96 s the
# keeper went on stamping PROGRESS onto a handle whose writer no longer existed,
# so handle-check answered HANDLE-INFLIGHT (rc 5) -- the reading a LIVE walk
# gives -- until an agent ran --exit by hand. At MAX_SECS=7200 that misreading
# has a two-HOUR ceiling, not a 96-second one.
# So: probe the writer BEFORE each stamp, and if it is gone, STOP. It must NOT
# stamp EXIT itself: deriving a verdict is exactly what this file is forbidden
# to do (stamp-handle.sh --exit is the only thing allowed to close a handle, and
# it re-derives rc/rows/holders from disk). UNKNOWN IS NOT DEAD -- if the probe
# errors or the row carries no numeric winpid, the keeper keeps going, the same
# rule stamp-handle.sh:68 already applies to the same tasklist probe.
#
# Usage: handle-keeper.sh <results-file> [interval-secs] [max-secs]
set -u
cd "${RAT_ROOT:-/c/afrisovera_OS}" || exit 70

RESULTS="${1:-}"
INTERVAL="${2:-20}"
MAX_SECS="${3:-28800}"   # 2.935: 7200 (2 h) was SHORTER THAN A REAL WALK -- the 2889 walk ran 6 h
[ -n "$RESULTS" ] || { echo "KEEPER-REFUSED (71): usage: handle-keeper.sh <results-file> [interval] [max-secs]"; exit 71; }
[ -f "${RESULTS}.handle" ] || { echo "KEEPER-REFUSED (72): no ${RESULTS}.handle; the keeper re-stamps, it never creates"; exit 72; }

# writer_state <handle> -> prints DEAD | ALIVE | UNKNOWN:<why>
# Reads the winpid off the handle's own WRITER row (never typed by a caller) and
# probes it with the SAME tasklist form stamp-handle.sh --exit uses.
writer_state() {
    local h="$1" wrow winpid live err
    wrow="$(grep -E '^WRITER	' "$h" 2>/dev/null | tr -d '\r' | tail -1 || true)"
    [ -n "$wrow" ] || { echo "UNKNOWN:no WRITER row"; return 0; }
    winpid="$(printf '%s\n' "$wrow" | tr '\t' '\n' | grep -m1 '^winpid=' | cut -d= -f2-)"
    case "${winpid:-}" in ''|*[!0-9]*) echo "UNKNOWN:no numeric winpid"; return 0 ;; esac
    err="$(mktemp)"
    live="$(MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${winpid}" 2>"$err")"
    if [ -s "$err" ]; then
        echo "UNKNOWN:probe errored: $(head -1 "$err")"; rm -f "$err"; return 0
    fi
    rm -f "$err"
    case "$live" in
        *"${winpid}"*) echo "ALIVE" ;;
        *)             echo "DEAD:winpid=${winpid}" ;;
    esac
}

START=$SECONDS
STAMPS=0
while [ $(( SECONDS - START )) -lt "$MAX_SECS" ]; do
    WSTATE="$(writer_state "${RESULTS}.handle")"
    case "$WSTATE" in
        DEAD:*)
            # The walk may have closed its own handle in the same breath; an EXIT
            # row means this is the ordinary end, not the defect.
            if grep -qE '^EXIT	' "${RESULTS}.handle" 2>/dev/null; then
                echo "KEEPER-DONE: writer is gone AND the handle carries an EXIT row, after ${STAMPS} stamp(s); the walk has ended"
                exit 0
            fi
            echo "KEEPER-STOPPED (65) after ${STAMPS} stamp(s): WRITER-GONE -- ${WSTATE#DEAD:} is dead and ${RESULTS}.handle has NO EXIT row; the keeper will not stamp PROGRESS onto a dead walk, and closing the handle is stamp-handle.sh --exit's job, not the keeper's"
            exit 65 ;;
    esac
    OUT="$(bash scripts/stamp-handle.sh --rows "$RESULTS" 2>&1)"; RC=$?
    case "$RC" in
        0)  STAMPS=$(( STAMPS + 1 )) ;;
        64) echo "KEEPER-DONE: handle CLOSED by --exit after ${STAMPS} stamp(s); the walk has ended"; exit 0 ;;
        *)  echo "KEEPER-STOPPED (${RC}) after ${STAMPS} stamp(s): ${OUT}"; exit "$RC" ;;
    esac
    sleep "$INTERVAL"
done
# 2.935: this branch used to print "handle left in-step" and exit 0. Both were
# FALSE. Expiry does not leave the handle in step -- it FREEZES it at the last
# stamp while the results file goes on growing, so handle-check.sh answers
# HANDLE-ROWS (7) on a perfectly healthy walk from that second onward, which is
# the exact defect 2.855 wrote this file to eliminate. MEASURED on the live 2889
# walk: keeper launched 16:25 local, expired 18:25 local after 334 stamps with
# rows=498; three hours later the file held 680 and handle-check.sh returned 7.
# rc 0 meant nothing downstream noticed. So: say what actually happened, and
# return a code a caller can see.
E_FROWS=0
[ -f "$RESULTS" ] && E_FROWS="$(wc -l < "$RESULTS" | tr -d ' ')"
E_HROWS="$(grep -E '^PROGRESS	' "${RESULTS}.handle" 2>/dev/null | tail -1 | tr '	' '
' | grep -m1 '^rows=' | cut -d= -f2-)"
echo "KEEPER-EXPIRED (66): ${MAX_SECS}s elapsed after ${STAMPS} stamp(s). The handle is NOT left in-step -- it FREEZES at rows=${E_HROWS:-?} while ${RESULTS} holds ${E_FROWS} and keeps growing, so handle-check.sh will answer HANDLE-ROWS (7) on a HEALTHY walk from now on. Re-launch a keeper (scripts/keeper-launch.sh) or the close-out will read a stale count."
exit 66
