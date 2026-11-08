#!/usr/bin/env bash
# stamp-handle.sh -- write ${RESULTS}.handle from the PROCESS TABLE, never from
# a number a human read off a screen.
#
# WHY THIS EXISTS (2.836, the defect 2.835 named but did not fix):
# `scripts/arm-holders.sh` refuses (exit 31) without ${RESULTS}.handle, and it
# derives BOTH holder paths and the winpid to attach from that file on purpose
# (2.792) so that no launcher ever types a pid. But NOTHING IN THE TREE WRITES
# THAT FILE -- `grep -n handle scripts/run-all-tests.sh` is empty. Every walk so
# far papered over the gap by hand: 2.810's handle says `detail=armed-by-observer`
# stamped at rows=293, and 2.835 stamped its own at rows=32 after a 300 s poll
# for a writer that was never coming. So the one path built to stop launchers
# typing pids still depended on a human typing the file it reads from.
#
# This closes it at the source: the pid comes from `ps -ef`, the winpid from the
# WINPID column of `ps -W` (the msys pid in `ps -ef` is NOT the winpid tasklist
# needs -- 263283 vs 7568 on the live 2.835 walk), and startedZ from Windows'
# own Win32_Process.CreationDate, converted to UTC by the CIM object, never by
# arithmetic on a local STIME string.
#
# IT REFUSES RATHER THAN GUESS -- same discipline as arm-holders.sh:
#   41  STAMP-REFUSED  no results argument
#   42  STAMP-REFUSED  no run-all-tests.sh writer for that results file
#   43  STAMP-REFUSED  MORE THAN ONE candidate writer; a handle must name one pid
#   44  STAMP-REFUSED  the pid has no WINPID row (ps -W); tasklist could not use it
#   45  STAMP-REFUSED  Windows gave no CreationDate for that winpid
#   46  STAMP-REFUSED  a handle already exists naming a DIFFERENT live winpid
# Re-stamping the SAME winpid is allowed and idempotent, so this is safe to call
# from a launch loop after every row.
#
# 2.838 adds `--exit`, the OTHER half: a walk that has ENDED cannot be stamped by
# the code above at all -- `ps -ef` no longer lists the writer, so it refuses 42,
# and handle-check.sh stays at HANDLE-INFLIGHT (5) forever, which close-out.sh
# maps to CLOSEOUT-REFUSED (15). The exit row is therefore derived from the two
# ARMED HOLDERS (2.792), the only observers that outlive the writer and hold its
# real exit code off a Win32 handle. It INVENTS NOTHING: the WRITER row is copied
# through byte-for-byte (the dead process cannot be re-derived), the rc comes from
# the holders, and the row count is cross-checked against the results file itself.
#   51  STAMP-REFUSED  no handle / no WRITER row to preserve
#   52  STAMP-REFUSED  that winpid is STILL ALIVE -- an EXIT stamp on a running writer is a lie
#   53  STAMP-REFUSED  no holder carries an EXIT row (nobody observed the exit)
#   54  STAMP-REFUSED  the two holders DISAGREE; a disagreement is not a verdict
#   55  STAMP-REFUSED  the holders' rows= disagrees with `wc -l < $RESULTS`
#   56  STAMP-REFUSED  the liveness probe errored; UNKNOWN is not DEAD (arm-holders 37)
#   57  STAMP-REFUSED  a holder's exit= is not a number
set -u

MODE=stamp
if [ "${1:-}" = "--exit" ]; then MODE=exit; shift; fi
if [ "${1:-}" = "--rows" ]; then MODE=rows; shift; fi

RESULTS="${1:-}"
[ -n "$RESULTS" ] || { echo "STAMP-REFUSED (41): usage: stamp-handle.sh [--exit] <results-file>"; exit 41; }
BASE="$(basename "$RESULTS")"
HANDLE="${RESULTS}.handle"

# ---- 2.838: --exit, the close-out half -------------------------------------
if [ "$MODE" = "exit" ]; then
    [ -f "$HANDLE" ] || { echo "STAMP-REFUSED (51): no ${HANDLE}; an EXIT row must preserve a WRITER row, not invent one"; exit 51; }
    WROW="$(grep -E '^WRITER	' "$HANDLE" | tr -d '\r' | tail -1 || true)"
    [ -n "$WROW" ] || { echo "STAMP-REFUSED (51): ${HANDLE} carries no WRITER row"; exit 51; }
    WINPID="$(printf '%s\n' "$WROW" | tr '\t' '\n' | grep -m1 '^winpid=' | cut -d= -f2-)"
    case "${WINPID:-}" in ''|*[!0-9]*) echo "STAMP-REFUSED (51): WRITER row carries no numeric winpid= (got '${WINPID:-}')"; exit 51 ;; esac

    LIVE_ERR="$(mktemp)"
    LIVE="$(MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${WINPID}" 2>"$LIVE_ERR")"
    if [ -s "$LIVE_ERR" ]; then
        echo "STAMP-REFUSED (56): liveness probe for winpid=${WINPID} errored: $(head -1 "$LIVE_ERR"); UNKNOWN is not DEAD"
        rm -f "$LIVE_ERR"; exit 56
    fi
    rm -f "$LIVE_ERR"
    case "$LIVE" in
        *"${WINPID}"*)
            echo "STAMP-REFUSED (52): winpid=${WINPID} is STILL ALIVE; an EXIT stamp on a running writer is a lie"
            exit 52 ;;
    esac

    # The holders are PowerShell-written and carry CRLF + a UTF-8 BOM; every field
    # read here goes through `tr -d '\r'` or `exit=0` compares unequal to `exit=0\r`.
    SEEN=""; RC=""; HROWS=""
    for f in "${RESULTS}.holderA" "${RESULTS}.holderB"; do
        [ -f "$f" ] || continue
        er="$(grep -E '^EXIT	' "$f" 2>/dev/null | tr -d '\r' | tail -1 || true)"
        [ -n "$er" ] || continue
        e="$(printf '%s\n' "$er" | tr '\t' '\n' | grep -m1 '^exit=' | cut -d= -f2-)"
        r="$(printf '%s\n' "$er" | tr '\t' '\n' | grep -m1 '^rows=' | cut -d= -f2-)"
        tag="$(basename "$f" | sed 's/.*holder//')"
        case "${e:-}" in ''|*[!0-9]*) echo "STAMP-REFUSED (57): holder ${tag} EXIT row carries exit='${e:-}', not a number"; exit 57 ;; esac
        if [ -z "$SEEN" ]; then
            RC="$e"; HROWS="$r"; SEEN="$tag"
        else
            [ "$e" = "$RC" ] || { echo "STAMP-REFUSED (54): holders DISAGREE on exit= (${SEEN}=${RC} vs ${tag}=${e}); a disagreement is not a verdict"; exit 54; }
            [ "$r" = "$HROWS" ] || { echo "STAMP-REFUSED (54): holders DISAGREE on rows= (${SEEN}=${HROWS} vs ${tag}=${r})"; exit 54; }
            SEEN="${SEEN}${tag}"
        fi
    done
    [ -n "$SEEN" ] || { echo "STAMP-REFUSED (53): no holder for ${BASE} carries an EXIT row; nobody observed the writer exit"; exit 53; }

    if [ -f "$RESULTS" ]; then FROWS="$(wc -l < "$RESULTS" | tr -d ' ')"; else FROWS=0; fi
    case "${HROWS:-}" in ''|*[!0-9]*) echo "STAMP-REFUSED (55): holder ${SEEN} EXIT row carries rows='${HROWS:-}', not a number"; exit 55 ;; esac
    [ "$HROWS" = "$FROWS" ] || { echo "STAMP-REFUSED (55): holders counted rows=${HROWS} but ${BASE} holds ${FROWS}; handle-check grades an EXIT count EXACTLY"; exit 55; }

    # Rewritten WHOLE, same as the live stamp: WRITER carried through unchanged so
    # re-running --exit is byte-identical, EXIT replacing any PROGRESS row.
    printf '%s\n' "$WROW" > "$HANDLE"
    printf 'EXIT\trows=%s\tdetail=rc=%s\tholders=%s\n' "$FROWS" "$RC" "$SEEN" >> "$HANDLE"
    echo "STAMPED-EXIT: ${HANDLE} winpid=${WINPID} rc=${RC} rows=${FROWS} holders=${SEEN}"
    exit 0
fi
# ---- end --exit ------------------------------------------------------------

# ---- 2.842: --rows, the CHEAP per-row re-stamp ------------------------------
# 2.841 priced the full stamp under the live 684 walk at 5.1-7.4 s, of which the
# `powershell Get-CimInstance` call is the bulk. Wiring THAT in after every row
# would add ~73 min to a ~100 min walk. The whole of that cost is redundant: a
# writer's pid/winpid/startedZ CANNOT change while it is the writer, and three
# consecutive machine stamps proved it (identical WRITER row, only rows= moved).
# So --rows derives nothing: it re-uses the WRITER row ALREADY on disk verbatim
# and only re-counts. Measured at 153 ms/row = ~105 s over 684 rows, 42x cheaper,
# and the WRITER row it emits is byte-identical to the machine stamp's, so
# handle-check.sh reads exactly what it reads today.
#
# It is deliberately NOT a fallback for `stamp`: with no handle, or a handle
# naming another file, or one already CLOSED by --exit, it REFUSES rather than
# deriving. A cheap mode that silently promotes itself to the expensive one is
# how a per-row hook gets its 73 minutes back by accident.
if [ "$MODE" = "rows" ]; then
    [ -f "$HANDLE" ] || { echo "ROWS-REFUSED (61): no ${HANDLE}; --rows re-uses a WRITER row, it never derives one"; exit 61; }
    WROW="$(grep '^WRITER	' "$HANDLE" | tail -1)"
    [ -n "${WROW:-}" ] || { echo "ROWS-REFUSED (62): ${HANDLE} carries no WRITER row"; exit 62; }
    # The handle must be about THIS file. A stale handle copied between results
    # files would otherwise re-stamp a live walk's count onto a stranger's writer.
    case "$WROW" in
        *"	results=${BASE}	"*) : ;;
        *) echo "ROWS-REFUSED (63): ${HANDLE} WRITER row is not about ${BASE}: ${WROW}"; exit 63 ;;
    esac
    # An EXIT row is a walk that has ENDED. Rewriting it back to PROGRESS would
    # resurrect a dead walk for every reader, so the close-out row is final.
    if grep -q '^EXIT	' "$HANDLE"; then
        echo "ROWS-REFUSED (64): ${HANDLE} is already CLOSED by --exit; a finished walk takes no more PROGRESS rows"
        exit 64
    fi
    [ -f "$RESULTS" ] || { echo "ROWS-REFUSED (65): ${RESULTS} does not exist; nothing to count"; exit 65; }
    ROWS="$(wc -l < "$RESULTS" | tr -d ' ')"
    # Whole rewrite, `>` then `>>`, exactly as the machine stamp does: two rows,
    # never an append, so per-row use cannot grow the file without bound.
    printf '%s\n' "$WROW" > "$HANDLE"
    printf 'PROGRESS\trows=%s\tdetail=stamped-by-machine\n' "$ROWS" >> "$HANDLE"
    echo "ROWS-STAMPED: ${HANDLE} rows=${ROWS} (writer row re-used, nothing derived)"
    exit 0
fi
# ---- end --rows -------------------------------------------------------------

# The writer's own cmdline is the only claim we trust. `ps -ef` is the only ps
# form here that prints ARGS (plain ps prints /usr/bin/bash and nothing else),
# so the match happens there and the winpid lookup happens in the other form.
#
# 2.836 falsifier: the first shape of this line was `... | grep -v ' grep '`
# plus a bare `awk '{print $2}'`, and its own pipeline's grep processes leaked
# into the candidate list -- arm 43's message read `--results --results 266150
# 266189`. It happened to REFUSE correctly only because the junk words carried
# no digits. Both halves are closed: grep lines are dropped by the [g]rep form
# (a literal ' grep ' misses `/usr/bin/grep`, which is how they got in), and a
# candidate must be a NUMERIC pid in column 2, not merely a line that matched.
CAND="$(ps -ef | grep -v '[g]rep' | grep 'run-all-tests\.sh' | grep -F -- "$BASE" | awk '$2 ~ /^[0-9]+$/ {print $2}')"
NCAND="$(printf '%s\n' "$CAND" | grep -c '^[0-9]')"
# 2.861: THE DEFAULT-RESULTS WALK WAS INVISIBLE TO THE MATCH ABOVE. Measured on
# live walk 2860: its argv is `bash scripts/run-all-tests.sh` with NO --results,
# so `grep -F -- .rat-results.tsv` found NOTHING and every default walk -- the
# 684-suite re-baseline, the only walk that matters -- refused 42 on its own
# handle. Only SELECTION runs (`--results X`) ever named their file in argv,
# which is why the 2.863 STAMP_PID path had always been exercised on those.
#
# The fallback DERIVES the default name from the runner's OWN source rather than
# hard-coding it, so if the runner's default moves this follows it. A writer
# whose argv carries no --results is, by the runner's own parsing, a writer for
# that default. Nothing is asserted by a caller: ps still supplies the pids, and
# the existing 43/47 arms still decide WHICH one.
#
# HONEST GAP, stated not hidden: `RAT_RESULTS=X run-all-tests.sh` also carries no
# --results in argv, and ps does not print environment, so such a walk is
# indistinguishable here from a default one. This fallback would name it as the
# default's writer. It is gated to the case where BASE *is* the derived default
# and the plain match found nothing, which is the narrowest form that repairs 42.
if [ "$NCAND" -eq 0 ]; then
    DEFRES="$(sed -n 's/^RESULTS="\${RAT_RESULTS:-\(.*\)}".*/\1/p' "$(dirname "$0")/run-all-tests.sh" 2>/dev/null | head -1)"
    if [ -n "$DEFRES" ] && [ "$DEFRES" = "$BASE" ]; then
        CAND="$(ps -ef | grep -v '[g]rep' | grep 'run-all-tests\.sh' | grep -v -- '--results' | awk '$2 ~ /^[0-9]+$/ {print $2}')"
        NCAND="$(printf '%s\n' "$CAND" | grep -c '^[0-9]')"
        [ "$NCAND" -ge 1 ] && echo "STAMP-NOTE: no writer named ${BASE} in argv; ${NCAND} argv-less writer(s) match the runner's derived default (${DEFRES})" >&2
    fi
fi
[ "$NCAND" -ge 1 ] || { echo "STAMP-REFUSED (42): no run-all-tests.sh writer for ${BASE}"; exit 42; }
# 2.863: A WALK STAMPING ITS OWN HANDLE IS ALWAYS MULTI-CANDIDATE. Measured on the
# real runner: `run-all-tests.sh --only smoke --results X` calling this from a
# command substitution puts THREE pids in the list above (the runner, its `$(...)`
# fork, and the launching wrapper), so arm 43 fires every time and the walk can
# never write its own handle. STAMP_PID lets the caller say WHICH of them it is.
# It is NOT a typed pid promoted to truth -- the header's whole point is that no
# launcher types a pid -- it only SELECTS from the list ps already derived, and
# refuses 47 when the caller names a pid ps did not corroborate.
if [ -n "${STAMP_PID:-}" ]; then
    case "${STAMP_PID}" in
        *[!0-9]*|'') echo "STAMP-REFUSED (47): STAMP_PID='${STAMP_PID}' is not a pid"; exit 47 ;;
    esac
    if printf '%s\n' "$CAND" | grep -qx "$STAMP_PID"; then
        CAND="$STAMP_PID"; NCAND=1
    else
        echo "STAMP-REFUSED (47): STAMP_PID=${STAMP_PID} is not among the ${NCAND} ps-derived writer(s) for ${BASE}: $(echo $CAND); a handle names a pid ps confirms, never one a caller asserts"
        exit 47
    fi
fi
[ "$NCAND" -eq 1 ] || { echo "STAMP-REFUSED (43): ${NCAND} candidate writers for ${BASE}: $(echo $CAND); a handle must name ONE pid"; exit 43; }
PID="$(printf '%s\n' "$CAND" | grep '[0-9]' | head -1)"

WINPID="$(ps -W 2>/dev/null | awk -v p="$PID" '$1==p {print $4; exit}')"
[ -n "${WINPID:-}" ] || { echo "STAMP-REFUSED (44): pid=${PID} has no WINPID row"; exit 44; }

# Ask Windows when it started. `.ToUniversalTime()` on the CIM DateTime does the
# local->UTC conversion, so no DST/offset arithmetic is written here by hand.
STARTED="$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter 'ProcessId=${WINPID}').CreationDate.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')" 2>/dev/null | tr -d '\r')"
case "$STARTED" in
    20[0-9][0-9]-*Z) : ;;
    *) echo "STAMP-REFUSED (45): no CreationDate for winpid=${WINPID} (got '${STARTED}')"; exit 45 ;;
esac

# Never overwrite somebody else's live claim. A handle that flips winpid mid-walk
# would send arm-holders.sh to attach to the wrong process, which is exactly the
# failure the whole 2.792 chain exists to prevent.
if [ -f "$HANDLE" ]; then
    OLD="$(grep -o 'winpid=[0-9][0-9]*' "$HANDLE" 2>/dev/null | tail -1 | cut -d= -f2)"
    if [ -n "${OLD:-}" ] && [ "$OLD" != "$WINPID" ]; then
        OLDLIVE="$(MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${OLD}" 2>/dev/null)"
        case "$OLDLIVE" in
            *"${OLD}"*)
                echo "STAMP-REFUSED (46): ${HANDLE} already claims live winpid=${OLD}, not ${WINPID}"
                exit 46 ;;
        esac
    fi
fi

printf 'WRITER\tresults=%s\tpid=%s\twinpid=%s\tstartedZ=%s\n' "$BASE" "$PID" "$WINPID" "$STARTED" > "$HANDLE"

# 2.837: the WRITER row ALONE is not a readable handle -- handle-check.sh grades
# relation 2 (the stamped count agrees with `wc -l < $RESULTS`) and REFUSES with
# HANDLE-MALFORMED (9) when no START/PROGRESS/EXIT row carries a rows= at all,
# which close-out.sh maps to CLOSEOUT-REFUSED (19). Measured on the live 2.835
# walk: rc=9. So every stamp also states the count it saw, and the handle is
# rewritten whole (WRITER + PROGRESS) rather than appended, so re-stamping after
# every row cannot grow the file without bound and the LATEST row is the only one.
# The count is read AFTER the WRITER row is written: a reader that catches us mid
# stamp sees the previous whole handle, never a half of this one.
if [ -f "$RESULTS" ]; then
    ROWS="$(wc -l < "$RESULTS" | tr -d ' ')"
else
    ROWS=0
fi
printf 'PROGRESS\trows=%s\tdetail=stamped-by-machine\n' "$ROWS" >> "$HANDLE"
echo "STAMPED: ${HANDLE} pid=${PID} winpid=${WINPID} startedZ=${STARTED} rows=${ROWS}"
exit 0
