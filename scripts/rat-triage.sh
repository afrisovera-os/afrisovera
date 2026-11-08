#!/usr/bin/env bash
# Triage for the regression registry's red suites: WHY is each one red?
#
# rat-progress.sh answers "how many are red and what are they called". It cannot
# answer the next question, and the next question is the one that decides whether a
# red suite is worth a milestone: a suite that lost because the thing it tests is
# broken and a suite that lost because its QEMU never came up are BOTH "FAIL" in the
# results file, and they are not remotely the same finding. Counting them together
# is how "9% red over 89 suites" becomes "~60 milestones of work waiting" when the
# true number may be a great deal smaller — and how a harness defect hides behind
# sixty apparent product defects.
#
# So this classifies each red by EVIDENCE the run already left on disk — 2.744's
# lesson that telling a real defect from a stale assertion is measurement and not
# reasoning, applied to the reds in bulk. Two artifacts per suite survive a run:
#   * the runner's own log,          $LOGDIR/rat-<suite>.log  (run-all-tests.sh:207)
#   * the suite's serial capture,    the file its own OUT= line names.
# The capture is the discriminator. A suite whose capture holds a full shell session
# and STILL failed its verdict made a claim that did not hold: that is a real finding
# and a real milestone. A suite whose capture is a connect traceback, or forty-odd
# bytes that stop in the middle of echoing its first command, never got as far as
# testing anything — the boot did not answer in time — and fixing the command under
# it would be fixing something that was never shown to be broken.
#
# Nothing is re-run and nothing is fixed. It reads artifacts only, so like its
# sibling it is safe to run WHILE a run is in progress — which is the point, since
# the run it triages takes hours and the reds are worth grouping before it ends.
#
# STALENESS IS CHECKED IN BOTH DIRECTIONS, because artifacts lie: a suite re-run by
# hand after the registry run overwrites its capture AND its log, and those are then
# evidence about the rerun and not about the run being triaged. A capture newer than
# the runner's log is reported STALE — and so is a LOG newer than the results row it
# explains, which the results file can now answer because the runner dates every row
# at completion. That second check is not hypothetical: a two-suite verification slice
# left a fresh log for a suite that PASSED in it, and this tool then filed that suite
# under ASSERTION/real-finding on the strength of its own passing log.
# A row with NO time (written before the column existed) cannot be checked at all, and
# is reported as exactly that — unknown, never silently counted as agreement.
#
# The registry is NOT duplicated here: suites and their capture paths are parsed out
# of run-all-tests.sh and the suite scripts themselves. A failed parse exits 2 rather
# than triaging a registry it guessed at.
#
# Usage:
#   scripts/rat-triage.sh                  # group every red by why it is red
#   scripts/rat-triage.sh --verdicts       # also print each suite's own verdict line
#   --results FILE (default .rat-results.tsv)   --min-capture N (default 120 bytes)
#   --logdir DIR (default .rat-logs, where the runner now writes both artifacts)
#
# EVIDENCE IS LOOKED FOR IN TWO PLACES, and a suite's log and capture always come
# from the SAME one. The runner used to write to the system temp dir, which prunes
# by age mid-run — that is why a full run's first 276 suites triaged as NO-EVIDENCE
# — so it now writes to --logdir. Runs recorded before that change left what they
# still have in ${TMPDIR:-/tmp}, and that is read as a fallback rather than thrown
# away; mixing the two per suite would compare a log from one run against a capture
# from another, which is exactly the staleness this refuses to classify.
#
# Exit 0 if nothing is red, 1 if anything is, 2 if it cannot do its job.
set -uo pipefail
cd "$(dirname "$0")/.."

RUNNER=scripts/run-all-tests.sh
RESULTS="${RAT_RESULTS:-.rat-results.tsv}"
LOGDIR="${RAT_LOGDIR:-.rat-logs}"      # where the runner writes; see the note above
LEGACY="${TMPDIR:-/tmp}"               # where runs before that change left theirs
MIN_CAPTURE="${RAT_MIN_CAPTURE:-120}"
VERDICTS=0

while [ $# -gt 0 ]; do
    case "$1" in
        --results) RESULTS="$2"; shift 2 ;;
        --min-capture) MIN_CAPTURE="$2"; shift 2 ;;
        --logdir) LOGDIR="$2"; shift 2 ;;
        --verdicts) VERDICTS=1; shift ;;
        -h|--help) sed -n "2,$(( $(grep -n '^set -uo pipefail' "$0" | head -1 | cut -d: -f1) - 1 ))p" "$0" |
                   sed 's/^# \?//'; exit 0 ;;
        *) echo "[rat-triage] unknown argument: $1 (try --help)" >&2; exit 2 ;;
    esac
done

case "$MIN_CAPTURE" in ''|*[!0-9]*)
    echo "[rat-triage] --min-capture must be a byte count, got '$MIN_CAPTURE'" >&2; exit 2 ;;
esac

# ---- the registry, from the runner itself: one list in the tree, not two ----
[ -f "$RUNNER" ] || { echo "[rat-triage] no $RUNNER — nothing defines the registry" >&2; exit 2; }
SUITES_LINE="$(grep -m1 '^SUITES=(' "$RUNNER")" ||
    { echo "[rat-triage] cannot find 'SUITES=(' in $RUNNER — refusing to guess the registry" >&2; exit 2; }
eval "$SUITES_LINE"
[ "${#SUITES[@]}" -gt 0 ] ||
    { echo "[rat-triage] parsed an EMPTY registry out of $RUNNER — refusing to report on it" >&2; exit 2; }

[ -f "$RESULTS" ] ||
    { echo "[rat-triage] no results file $RESULTS — run scripts/run-all-tests.sh first" >&2; exit 2; }

# ---- results: last row for a suite wins, exactly as the runner reads them back ----
# The fifth column is the row's completion time. It is what makes "is this evidence
# about THIS row?" answerable at all: a row is otherwise suite/status/seconds/exit-code
# and carries no time, so a log left by a later hand-rerun looks exactly like the log
# the row was born with. Rows written before the column existed have no time; they are
# held as UNKNOWN and reported as unknown, never quietly counted as "checked".
declare -A ST
declare -A ROWTIME
while IFS=$'\t' read -r s st sec rc ts; do
    [ -n "${s:-}" ] || continue
    ST[$s]="${st:-?}"
    case "${ts:-}" in ''|*[!0-9]*) ROWTIME[$s]="" ;; *) ROWTIME[$s]="$ts" ;; esac
done < "$RESULTS"

# ---- the capture path is the suite's own, parsed from the suite, not assumed ----
# Most suites write ${TMPDIR:-/tmp}/<something>.out, but <something> is not always the
# suite name, so guessing it would silently classify the wrong file.
capture_of() {
    local script="scripts/$1-test.sh" line
    [ -f "$script" ] || return 1
    line="$(grep -m1 '^OUT=' "$script")" || return 1
    printf '%s' "$line" | sed -n 's|.*/\([A-Za-z0-9_.:@+-]*\.out\).*|\1|p'
}

mtime_of() { stat -c%Y "$1" 2>/dev/null; }

# ---- which directory holds this suite's evidence: the runner's log decides ----
# Both artifacts are then read from that ONE directory, so a log from the current
# run is never weighed against a capture left behind by an older one.
evdir_of() {
    local d
    for d in "$LOGDIR" "$LEGACY"; do
        [ -f "${d}/rat-${1}.log" ] && { printf '%s' "$d"; return 0; }
    done
    return 1
}

declare -A GROUP
declare -A VERDICT
reds=0
undated=0     # reds whose row predates the completion-time column: unknown, not agreement

for s in "${SUITES[@]}"; do
    st="${ST[$s]:-NOT-RUN}"
    # NOT-RUN is owed, not red: it is the run's remainder and rat-progress.sh prices it.
    case "$st" in PASS|NOT-RUN) continue ;; esac
    reds=$((reds + 1))

    # MISSING is decided by the registry, not by artifacts: there is no script to run.
    if [ "$st" = MISSING ]; then
        GROUP[REGISTRY/missing-script]="${GROUP[REGISTRY/missing-script]:-}${s} "
        continue
    fi

    if ! ed="$(evdir_of "$s")"; then
        GROUP[NO-EVIDENCE/log-gone]="${GROUP[NO-EVIDENCE/log-gone]:-}${s} "
        continue
    fi
    log="${ed}/rat-${s}.log"

    # EVIDENCE MUST NOT POSTDATE THE ROW IT EXPLAINS. The runner dates each row when
    # the suite completes, i.e. after its last write to the log — so a log NEWER than
    # the row was written by a later run, and classifying the row from it reports on a
    # run nobody asked about. This is the mirror of the capture-vs-log check below, and
    # it is the one that was missing: a hand-rerun in which a suite PASSED once put that
    # suite in ASSERTION/real-finding on the strength of its own passing log.
    lm="$(mtime_of "$log")"
    rt="${ROWTIME[$s]:-}"
    if [ -n "$rt" ] && [ -n "$lm" ] && [ "$lm" -gt "$rt" ]; then
        GROUP[STALE/after-the-row]="${GROUP[STALE/after-the-row]:-}${s} "
        continue
    fi
    # No row time at all: the check above could not run. Say so rather than implying it passed.
    [ -n "$rt" ] || undated=$((undated + 1))

    VERDICT[$s]="$(grep -aE 'TEST (FAILED|PASSED)|FAILED' "$log" | tail -1 | sed 's/^[[:space:]]*//')"

    cap="$(capture_of "$s")" || cap=""
    if [ -z "$cap" ] || [ ! -f "${ed}/${cap}" ]; then
        # No serial capture to weigh: a different harness shape (or it never wrote one).
        GROUP[NO-CAPTURE/read-by-hand]="${GROUP[NO-CAPTURE/read-by-hand]:-}${s} "
        continue
    fi
    capf="${ed}/${cap}"

    # Artifacts lie once a suite is re-run by hand. A capture newer than the runner's
    # log — or newer than the row itself, when the row is dated — is evidence about the
    # rerun, so it is not classified at all.
    cm="$(mtime_of "$capf")"
    if [ -n "$cm" ] && { { [ -n "$lm" ] && [ "$cm" -gt "$lm" ]; } ||
                         { [ -n "$rt" ] && [ "$cm" -gt "$rt" ]; }; }; then
        GROUP[STALE/capture-overwritten]="${GROUP[STALE/capture-overwritten]:-}${s} "
        continue
    fi

    size="$(stat -c%s "$capf" 2>/dev/null || echo 0)"
    if grep -aqE 'ConnectionRefusedError|ConnectionResetError|Traceback \(most recent call last\)' "$capf"; then
        # Never connected: QEMU was not listening by the time the suite's settle slept out.
        GROUP[HARNESS/no-connect]="${GROUP[HARNESS/no-connect]:-}${s} "
    elif [ "$size" -lt "$MIN_CAPTURE" ]; then
        # Connected, then the session stopped dead — typically mid-echo of command one.
        GROUP[HARNESS/short-capture]="${GROUP[HARNESS/short-capture]:-}${s} "
    else
        # A full session, and the suite still judged itself LOST. This is the real kind.
        GROUP[ASSERTION/real-finding]="${GROUP[ASSERTION/real-finding]:-}${s} "
    fi
done

count_of() { local n=0 _x; for _x in ${1:-}; do n=$((n + 1)); done; printf '%s' "$n"; }

ORDER=(ASSERTION/real-finding HARNESS/no-connect HARNESS/short-capture
       NO-CAPTURE/read-by-hand REGISTRY/missing-script NO-EVIDENCE/log-gone
       STALE/capture-overwritten STALE/after-the-row)

echo
echo "============ registry run: why each red suite is red (${RESULTS}) ============"
if [ "$reds" -eq 0 ]; then
    echo "  no red suites recorded — nothing to triage"
    echo "=========================================================================="
    exit 0
fi
printf '  %d red suites, grouped by the evidence the run left on disk\n' "$reds"
echo "  --------------------------------------------------------------"

for g in "${ORDER[@]}"; do
    names="${GROUP[$g]:-}"; [ -n "$names" ] || continue
    n="$(count_of "$names")"
    case "$g" in
        ASSERTION/real-finding)
            note="full session captured, verdict still LOST — a real finding, queue it" ;;
        HARNESS/no-connect)
            note="capture is a connect traceback — QEMU not listening yet, tests nothing" ;;
        HARNESS/short-capture)
            note="capture under ${MIN_CAPTURE}B, dies mid-command — boot never answered" ;;
        NO-CAPTURE/read-by-hand)
            note="no serial capture to weigh — different harness shape, read it" ;;
        REGISTRY/missing-script)
            note="registered with no script on disk — a registry defect, not a test one" ;;
        NO-EVIDENCE/log-gone)
            note="runner log gone — re-run the suite to get evidence back" ;;
        STALE/capture-overwritten)
            note="capture is NEWER than the run — evidence about a rerun, not this run" ;;
        STALE/after-the-row)
            note="log is NEWER than the results row — re-runs the row, re-record it" ;;
    esac
    printf '  %-26s %4d   %s\n' "$g" "$n" "$note"
    printf '                             %s\n' $names
    if [ "$VERDICTS" = 1 ]; then
        for s in $names; do
            [ -n "${VERDICT[$s]:-}" ] && printf '                               %s: %s\n' "$s" "${VERDICT[$s]}"
        done
    fi
done

harness=$(( $(count_of "${GROUP[HARNESS/no-connect]:-}") + $(count_of "${GROUP[HARNESS/short-capture]:-}") ))
real=$(count_of "${GROUP[ASSERTION/real-finding]:-}")
echo "  --------------------------------------------------------------"
printf '  %d of %d reds are the harness, not the tree; %d made a claim that did not hold\n' \
    "$harness" "$reds" "$real"
echo "  a red in a HARNESS group is not evidence the command under it is broken:"
echo "  re-run that one suite alone before spending a milestone on it."
if [ "$undated" -gt 0 ]; then
    # Not a footnote: these classifications rest on evidence that COULD NOT be checked
    # against the run it is supposed to explain, and saying so is the whole point.
    printf '  %d of them were classified from an UNDATED row (no completion time in %s):\n' \
        "$undated" "$RESULTS"
    echo "  their evidence could not be checked against the run — unknown, not agreement."
    echo "  re-run those suites (--failed) so the row and its evidence share a run."
fi
echo "=========================================================================="

# Red is red whatever the reason — the exit code does not get to be optimistic just
# because the cause turned out to be the harness. Only an all-green run exits 0.
[ "$reds" -eq 0 ]
