#!/usr/bin/env bash
# keeper-launch.sh -- the LAUNCHER handle-keeper.sh never had (2.857 gap (c)).
#
# WHY THIS EXISTS: handle-keeper.sh has shipped since 2.855 and every milestone
# since has closed with the same honest gap -- `grep -rln handle-keeper` finds
# only the file itself. A keeper that nothing ever starts keeps nothing: the
# handle still goes stale mid-walk (the 2.835 reading: handle rows=255 while the
# file held 678, handle-check -> HANDLE-ROWS (7), which close-out maps to a
# refusal). This file is the one thing missing: it BACKGROUNDS the keeper beside
# a running walk and PROVES the first stamp landed before it returns.
#
# It derives NOTHING about the walk: every stamp is still handle-keeper.sh ->
# stamp-handle.sh --rows, which re-uses the WRITER row on disk verbatim. This
# file only decides that a keeper should exist, and refuses when one should not:
#   71 no results file named
#   72 no ${RESULTS}.handle -- the keeper re-stamps, it never creates one
#   73 a keeper is ALREADY live for this handle (two keepers = double PROGRESS)
#   74 the handle is already CLOSED by --exit -- a finished walk takes no keeper
#   75 launched but no first stamp landed inside the grace window (log printed)
#
# LIVENESS NOTE (2.765 rule 5 hazard, named not repeated): the pidfile check
# uses `kill -0` on the keeper's MSYS pid, which is OUR OWN CHILD in OUR OWN
# bash universe -- not a Windows winpid off a handle row. The falsified probe
# was `kill -0 <WINPID>`; this is not that, and it must never be pointed at one.
#
# Usage: keeper-launch.sh <results-file> [interval-secs] [max-secs]
set -u
cd "${RAT_ROOT:-/c/afrisovera_OS}" || exit 70

RESULTS="${1:-}"
INTERVAL="${2:-20}"
MAX_SECS="${3:-28800}"   # 2.935: matched to handle-keeper.sh; 2 h expired mid-walk on 2889
[ -n "$RESULTS" ] || { echo "LAUNCH-REFUSED (71): usage: keeper-launch.sh <results-file> [interval] [max-secs]"; exit 71; }

HANDLE="${RESULTS}.handle"
PIDF="${RESULTS}.keeper.pid"
LOG="${RESULTS}.keeper.log"

[ -f "$HANDLE" ] || { echo "LAUNCH-REFUSED (72): no ${HANDLE}; stamp the walk's handle first (stamp-handle.sh ${RESULTS}) -- the keeper re-stamps, it never creates"; exit 72; }

if grep -qE '^EXIT	' "$HANDLE" 2>/dev/null; then
    echo "LAUNCH-REFUSED (74): ${HANDLE} is already CLOSED by --exit; a finished walk takes no keeper"
    exit 74
fi

if [ -f "$PIDF" ]; then
    OLD="$(tr -d '\r\n' < "$PIDF")"
    case "${OLD:-}" in
        ''|*[!0-9]*) : ;;                       # unreadable pidfile -> treat as stale
        *) if kill -0 "$OLD" 2>/dev/null; then
               echo "LAUNCH-REFUSED (73): keeper pid ${OLD} is already live for ${HANDLE} (${PIDF}); two keepers would double every PROGRESS row"
               exit 73
           fi ;;
    esac
    echo "[keeper-launch] stale ${PIDF} (pid ${OLD:-none} is gone); replacing"
fi

# 2.863, MEASURED against a REAL stamped handle (2.862 only ever saw a hand-built
# one carrying a bare WRITER row): counting PROGRESS rows CANNOT witness a stamp.
# stamp-handle.sh rewrites the handle WHOLE (WRITER + one PROGRESS), so the count
# is pinned at 1 from the first machine stamp onward, and the row itself is
# `rows=N detail=stamped-by-machine` with no clock in it -- when the walk is
# between rows the new bytes are IDENTICAL to the old. The only thing that always
# moves on a successful stamp is the REWRITE itself, so the signature is
# count + row text + mtime, and any one of the three changing is the evidence.
handle_sig() {
    printf '%s|%s|%s' \
        "$(grep -cE '^PROGRESS	' "$HANDLE" 2>/dev/null || true)" \
        "$(grep -E '^PROGRESS	' "$HANDLE" 2>/dev/null | tail -1)" \
        "$(stat -c %Y "$HANDLE" 2>/dev/null || true)"
}
BEFORE_SIG="$(handle_sig)"
BEFORE="$(grep -cE '^PROGRESS	' "$HANDLE" 2>/dev/null || true)"; BEFORE="${BEFORE:-0}"

nohup bash scripts/handle-keeper.sh "$RESULTS" "$INTERVAL" "$MAX_SECS" > "$LOG" 2>&1 &
KPID=$!
printf '%s\n' "$KPID" > "$PIDF"

# Prove the first stamp landed. A keeper that dies on its first call is a
# launcher that lied, so this waits for EVIDENCE (a new PROGRESS row) or for the
# child to exit -- never just for the clock.
GRACE=15
i=0
while [ "$i" -lt "$GRACE" ]; do
    NOW_SIG="$(handle_sig)"
    if [ "$NOW_SIG" != "$BEFORE_SIG" ]; then
        NOW="$(grep -cE '^PROGRESS	' "$HANDLE" 2>/dev/null || true)"; NOW="${NOW:-0}"
        echo "KEEPER-LAUNCHED: pid=${KPID} handle=${HANDLE} interval=${INTERVAL}s max=${MAX_SECS}s; first stamp landed (PROGRESS ${BEFORE} -> ${NOW}, handle rewritten); log=${LOG}"
        exit 0
    fi
    if ! kill -0 "$KPID" 2>/dev/null; then
        echo "LAUNCH-REFUSED (75): keeper pid ${KPID} exited before landing a stamp; its log says:"
        sed -n '1,20p' "$LOG"
        rm -f "$PIDF"
        exit 75
    fi
    sleep 1
    i=$(( i + 1 ))
done

echo "LAUNCH-REFUSED (75): keeper pid ${KPID} is alive but landed no PROGRESS row in ${GRACE}s (PROGRESS still ${BEFORE}); its log says:"
sed -n '1,20p' "$LOG"
exit 75
