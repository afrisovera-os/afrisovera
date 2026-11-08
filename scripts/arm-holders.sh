#!/usr/bin/env bash
# scripts/arm-holders.sh -- ONE PATH FOR THE EXIT-CODE HOLDERS (2.792 gap 1).
#
# Usage:
#   ./scripts/arm-holders.sh --paths <results.tsv>   # print the two canonical paths, arm nothing
#   ./scripts/arm-holders.sh <results.tsv>           # arm BOTH holders at those paths
#
# THE GAP THIS CLOSES. Every walk from 2.765 to 2.790 named its holder files by
# hand, per milestone, in the launcher: .rat-verdict-2765.tsv, then
# .rat-verdict-2766b/c.tsv, then .rat-verdict-2790a/b.tsv -- and on the 2790 walk
# a SECOND generation (.rat-holderA-2790.out / .rat-holderB-2790.out, armed ~12
# rows later by a different hand) existed at the same time. FOUR holder files,
# two generations, two schemas: the 2790 pair omitted `holder=<pid>` from its
# ATTACHED row, which is a field .rat-walk-2766.sh reads. The verdict script and
# the arming step could therefore disagree about which pair is graded, and the
# only thing keeping them honest was a paragraph of prose in .rat-final-2790.sh.
#
# THE FIX IS 2.792'S, APPLIED TO THE HOLDERS. The writer already stamps its own
# pid into a handle DERIVED FROM $RESULTS (${RESULTS}.handle), so there is no
# launcher-supplied path to get wrong. Do the same here:
#
#     holder A = ${RESULTS}.holderA        holder B = ${RESULTS}.holderB
#
# and take the pid to attach to FROM THE WRITER'S OWN HANDLE, never from an
# argument. A launcher that cannot type a path cannot type the wrong one, and a
# launcher that cannot type a pid cannot arm a holder on a process that is not
# the walk (2.791's finding: two holders cross-agreeing about a pid that was not
# the one appending rows).
#
# ONE IMPLEMENTATION, NOT TWO (2.765 rule 5). It execs .rat-verdict-2765.ps1
# UNEDITED -- the schema .rat-final-2766.sh was falsified 5/5 against, the one
# that writes `holder=$PID`. The 2790 generation is not re-created here; if the
# grader only ever sees this script's output, there is only one schema left.
#
# WHY scripts/ AND NOT .rat-* (2.793): .gitignore swallows the whole .rat-*
# namespace, so an arming step written there is untracked and the next agent
# cannot find it. The thing that removes the choice must be the thing that ships.
#
# IT REFUSES RATHER THAN GUESS. No handle, an unreadable handle, a dead writer,
# or holder files that already exist are each a refusal -- never an arm on a
# guessed pid, and never a clobber of evidence already on disk.
#
# EXIT CODES -- the 3x band is this arming step, and overlaps nothing above:
#   0   ARMED        both holders launched and their ATTACHED rows observed
#   30  ARM-REFUSED  no results file given, or it does not exist
#   31  ARM-REFUSED  no ${RESULTS}.handle -- this walk cannot say who wrote it
#   32  ARM-REFUSED  the handle carries no readable winpid= (pre-2.792 shape)
#   33  ARM-REFUSED  that winpid is not alive -- there is nothing left to hold
#   34  ARM-REFUSED  holder files already exist; arming would overwrite evidence
#   35  ARM-REFUSED  the holder implementation is missing
#   36  ARM-REFUSED  launched, but an ATTACHED row never appeared (also: registry
#                    size not corroborated)
#   37  ARM-REFUSED  the liveness probe itself ERRORED (2.833) -- UNKNOWN, and
#                    UNKNOWN is not DEAD, so nothing is armed and nothing is
#                    declared a corpse
#   38  ARM-REFUSED  the handle carries no startedZ= -- cannot rule out PID reuse
#   39  ARM-REFUSED  winpid is alive, but its CreationDate does not match the
#                    handle's startedZ -- PID REUSE: some OTHER process now
#                    holds this number, not the walk the handle names
set -u

HOLDER_PS1="${RAT_HOLDER_PS1:-.rat-verdict-2765.ps1}"
ATTACH_WAIT_S="${RAT_ARM_WAIT:-20}"

# THE ONE PLACE THE PATHS ARE SPELLED. Readers call `--paths` instead of typing.
holder_paths() { printf '%s.holderA %s.holderB\n' "$1" "$1"; }

RESULTS="${1:-}"
if [ "$RESULTS" = "--paths" ]; then
    shift
    RESULTS="${1:-}"
    [ -n "$RESULTS" ] || { echo "ARM-REFUSED (30): --paths needs a results file"; exit 30; }
    holder_paths "$RESULTS"
    exit 0
fi

# 2.859: PROBE-ONLY MODE, arming nothing. A DISJOINT grammar beside --paths, not a
# change to the arm path: every existing invocation reaches the same code it did.
# It exists so run-all-tests.sh's startup guard can ask "does this handle name a
# LIVE writer?" WITHOUT a second copy of the tasklist form below -- 2.765 rule 5,
# and that form is the one scripts/liveprobe-falsify-2831.sh falsified. It shares
# refusals 31/32/37 verbatim and returns 0 only where the arm path would proceed.
LIVE_ONLY=0
if [ "$RESULTS" = "--live" ]; then
    shift
    LIVE_ONLY=1
    RESULTS="${1:-}"
    [ -n "$RESULTS" ] || { echo "ARM-REFUSED (30): --live needs a results file"; exit 30; }
fi

[ -n "$RESULTS" ] || { echo "ARM-REFUSED (30): no results file given"; exit 30; }
case "$RESULTS" in
    /*) echo "ARM-REFUSED (30): give a path relative to the project root, not ${RESULTS}"; exit 30 ;;
esac
# NOT `[ -f "$RESULTS" ]`. MEASURED 2.796, on a real walk: the writer stamps its
# handle BEFORE the first row lands, so at row 0/684 the results file does not
# exist yet -- and row 0 is exactly where 2.768's holders were armed, which is
# the only reason that walk survived its launcher dying. Requiring the results
# file here refused a perfectly live walk (rc=30 at 36 s, handle already on
# disk). THE HANDLE IS THE AUTHORITY (2.792), not the artefact it describes; the
# holder reads rows with -ErrorAction SilentlyContinue and counts 0 until the
# file appears. A missing results file is now arm 31's problem (no handle) or no
# problem at all.
HANDLE="${RESULTS}.handle"
[ -f "$HANDLE" ] || { echo "ARM-REFUSED (31): no ${HANDLE}; nothing here claims a pid"; exit 31; }

# The writer re-stamps the handle after every row, so the LAST winpid= is the
# live one. 2.795: pre-fix handles carry a subshell's winpid -- a stale number
# reads as "not alive" below and is refused, not armed on.
WINPID="$(grep -o 'winpid=[0-9][0-9]*' "$HANDLE" 2>/dev/null | tail -1 | cut -d= -f2)"
[ -n "$WINPID" ] || { echo "ARM-REFUSED (32): ${HANDLE} carries no winpid="; exit 32; }

# 2.833 repair; falsifier scripts/liveprobe-falsify-2831.sh cell 2. The old form
# was `tasklist //FI "PID eq $WINPID" 2>/dev/null | grep -q`. With
# MSYS_NO_PATHCONV exported by the CALLER, `//FI` reaches tasklist literally,
# tasklist errors, the error lands in /dev/null and the empty stdout reads as
# "not alive" -- a LIVE walk called a corpse, the family's flattering direction.
# Both halves are closed: the switch is `/FI` with MSYS_NO_PATHCONV set for THIS
# command only (so the caller's env cannot change the answer), and stderr is
# captured, so a probe that ERRORED is UNKNOWN (exit 37 -- 36 was already taken by the ATTACH timeout) and never DEAD.
LIVE_ERR="$(mktemp)"
LIVE_OUT="$(MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${WINPID}" 2>"$LIVE_ERR")"
if [ -s "$LIVE_ERR" ]; then
    echo "ARM-REFUSED (37): liveness probe for winpid=${WINPID} errored: $(head -1 "$LIVE_ERR"); UNKNOWN is not DEAD"
    rm -f "$LIVE_ERR"
    exit 37
fi
rm -f "$LIVE_ERR"
case "$LIVE_OUT" in
    *"${WINPID}"*) : ;;
    *)
        echo "ARM-REFUSED (33): winpid=${WINPID} is not alive; a handle cannot be taken on a corpse"
        exit 33
        ;;
esac

# PID-REUSE GUARD (3.7xx). `tasklist` only proves SOME process holds this pid
# right NOW -- not that it is the SAME process the handle was stamped for.
# Windows recycles pids; a walk that died minutes ago can have its winpid
# handed to an unrelated process before this script ever runs, and the bare
# alive-check above reads that impostor as the walk. stamp-handle.sh's
# startedZ (Win32_Process.CreationDate, converted to UTC by the CIM object --
# see scripts/stamp-handle.sh line ~219) is the one fact an impostor process
# cannot share, so cross-check the CURRENT holder's CreationDate against it
# before trusting the tasklist match above. A live process's CreationDate is
# invariant, and both sides are derived by the identical CIM call+format, so
# an exact string match is the right comparison -- no clock-skew tolerance to
# get wrong.
STARTEDZ="$(grep -o 'startedZ=[^	]*' "$HANDLE" 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\r')"
[ -n "$STARTEDZ" ] || { echo "ARM-REFUSED (38): ${HANDLE} carries no startedZ=; cannot rule out PID reuse"; exit 38; }

CIM_ERR="$(mktemp)"
CIM_STARTED="$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter 'ProcessId=${WINPID}').CreationDate.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')" 2>"$CIM_ERR" | tr -d '\r')"
if [ -s "$CIM_ERR" ] || [ -z "$CIM_STARTED" ]; then
    echo "ARM-REFUSED (37): CreationDate probe for winpid=${WINPID} errored/empty: $(head -1 "$CIM_ERR" 2>/dev/null); UNKNOWN is not DEAD"
    rm -f "$CIM_ERR"
    exit 37
fi
rm -f "$CIM_ERR"
if [ "$CIM_STARTED" != "$STARTEDZ" ]; then
    echo "ARM-REFUSED (39): winpid=${WINPID} is alive but CreationDate=${CIM_STARTED} != handle's startedZ=${STARTEDZ} -- PID REUSE, not the walk this handle names"
    exit 39
fi

# 2.859: --live stops HERE, one line past the probe, having armed nothing. Reaching
# this point means the handle exists, carries a winpid, tasklist LISTED it, and
# its CreationDate matches the handle's startedZ (not a pid-reuse impostor).
if [ "$LIVE_ONLY" = 1 ]; then
    echo "LIVE: ${HANDLE} names winpid=${WINPID}, which tasklist lists as running"
    exit 0
fi

set -- $(holder_paths "$RESULTS")
VA="$1"; VB="$2"
# 2.909: a blanket `[ -e ]` gate turns ONE dead walk into a PERMANENT disarm.
# Measured at 2.907: both holders refused 34 on files left by the 03:39 walk
# that died at 04:46, so a brand-new walk could arm NEITHER exit-code witness
# until a human moved them by hand. Ask the decider instead -- STALE means
# "EXIT row present OR the pid it names is DEAD" -- and MOVE ASIDE, never
# delete: the corpse is the evidence for why the previous walk ended. Only
# rc=0 may move; 10 LIVE / 12 / 13 UNKNOWN all keep refusing 34, so a holder
# still witnessing a LIVE walk is never clobbered.
STALE_CHK="$(dirname "$0")/holder-stale-check.sh"
for f in "$VA" "$VB"; do
    [ -e "$f" ] || continue
    if [ -f "$STALE_CHK" ]; then
        _verdict="$(bash "$STALE_CHK" "$f" 2>&1)"; _rc=$?
    else
        _rc=99; _verdict="no decider at ${STALE_CHK}"
    fi
    if [ "$_rc" != 0 ]; then
        echo "ARM-REFUSED (34): ${f} exists and is not stale (rc=${_rc}): ${_verdict}"
        exit 34
    fi
    _suffix=".stale$(date -u +%Y%m%dT%H%M%SZ)"
    for g in "$f" "${f}.log"; do
        [ -e "$g" ] || continue
        mv -f "$g" "${g}${_suffix}" || {
            echo "ARM-REFUSED (34): ${g} is stale but could not be moved aside"; exit 34; }
    done
    echo "[arm-holders] moved aside stale ${f} -> ${f}${_suffix}: ${_verdict}"
done

[ -f "$HOLDER_PS1" ] || { echo "ARM-REFUSED (35): no holder implementation at ${HOLDER_PS1}"; exit 35; }

# `--list` prints ONE banner line ("[run-all] N suites registered; ...") ahead of the
# indented suite names, so `| wc -l` is the registry size PLUS ONE. 2.797's holders were
# armed off that line count and carry `rows=N/685` against a 684-suite registry -- a
# complete walk will stamp `rows=684/685`, reading ONE SHORT on a walk that is not short.
# Fixed the way 2.761 intended when it put the count IN the banner: two derivations off
# the SAME single capture (the banner STATES the count; the indented names are COUNTED),
# and a refusal if they disagree. One number with nothing to check it against is exactly
# what produced 685. `--list` is captured ONCE -- it is slow under a live walk.
if [ -n "${RAT_EXPECT:-}" ]; then
    EXPECT="$RAT_EXPECT"
else
    _list="$(bash scripts/run-all-tests.sh --list 2>/dev/null)"
    _banner="$(printf '%s\n' "$_list" | sed -n 's/^\[run-all\] \([0-9][0-9]*\) suites registered.*/\1/p' | head -1)"
    _names="$(printf '%s\n' "$_list" | awk '/^  [^ ]+$/ { n[$1]=1 } END { print length(n) }')"
    if [ -z "$_banner" ] || [ "$_banner" != "$_names" ]; then
        echo "ARM-REFUSED (36): registry size not corroborated -- banner='${_banner}' names='${_names}'"
        exit 36
    fi
    EXPECT="$_banner"
fi
RUNLOG="${RAT_RUNLOG:-${RESULTS}.log}"

echo "== arming two holders on winpid=${WINPID} (read off ${HANDLE}, not typed) =="
echo "holder A = ${VA}"
echo "holder B = ${VB}"

for f in "$VA" "$VB"; do
    nohup powershell -NoProfile -File "$HOLDER_PS1" \
        -WinPid "$WINPID" -Out "$f" -RunLog "$RUNLOG" \
        -Results "$RESULTS" -Expect "$EXPECT" > "${f}.log" 2>&1 &
done

# ARMED IS NOT LAUNCHED. A holder only matters once it has taken the handle, and
# the ATTACHED row is the only evidence that it did (2.792's live arm).
i=0
while [ "$i" -lt "$ATTACH_WAIT_S" ]; do
    if grep -q 'ATTACHED' "$VA" 2>/dev/null && grep -q 'ATTACHED' "$VB" 2>/dev/null; then
        echo "ARMED: both holders ATTACHED to winpid=${WINPID}"
        grep -h 'ATTACHED' "$VA" "$VB"
        echo "export RAT_VA='${VA}' RAT_VB='${VB}' RAT_VERDICTS='${VA} ${VB}'"
        exit 0
    fi
    sleep 1
    i=$((i + 1))
done

echo "ARM-REFUSED (36): no ATTACHED row after ${ATTACH_WAIT_S}s; see ${VA}.log / ${VB}.log"
exit 36
