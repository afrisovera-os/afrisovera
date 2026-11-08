#!/usr/bin/env bash
# Statement of account for the regression registry run.
#
# `run-all-tests.sh --rollup` answers "is the tree green?" and answers it well, but
# it rolls every not-green state into ONE number ("N not green") — and the whole
# thesis of 2.743 is that FAIL, TIMEOUT, DIED, NO-VERDICT, MISSING and NOT-RUN are
# SIX DIFFERENT STATES and only one of them means "this suite ran and lost". This
# breaks them out, names every suite in each, and never counts any of them as a pass.
#
# The second thing it does is make the run PAYABLE. 673 suites at ~19s of QEMU boot
# apiece is hours of wall-clock, which is why --resume exists; an instalment plan
# needs a balance, a rate and a remainder, so this prints them from the seconds the
# results file already records. Safe to run WHILE a run is in progress — it only
# reads the TSV, so it is the way to see how far tonight's instalment got.
#
# The registry is NOT duplicated here: it is parsed out of run-all-tests.sh, so there
# is exactly one list of suites in the tree and this cannot drift green while that
# one grows. If the parse fails, this exits non-zero rather than reporting on a
# registry it guessed at.
#
# Usage:
#   scripts/rat-progress.sh              # breakdown + rate + ETA
#   scripts/rat-progress.sh --names      # also name every suite in every state
#   --results FILE (default .rat-results.tsv)   --notrun-list N (default 10)
#
# Exit 0 ONLY if the registry is fully walked AND everything in it is PASS.
set -uo pipefail
cd "$(dirname "$0")/.."

RUNNER=scripts/run-all-tests.sh
RESULTS="${RAT_RESULTS:-.rat-results.tsv}"
NOTRUN_LIST="${RAT_NOTRUN_LIST:-10}"
NAMES=0

while [ $# -gt 0 ]; do
    case "$1" in
        --results) RESULTS="$2"; shift 2 ;;
        --notrun-list) NOTRUN_LIST="$2"; shift 2 ;;
        --names) NAMES=1; shift ;;
        -h|--help) sed -n "2,$(( $(grep -n '^set -uo pipefail' "$0" | head -1 | cut -d: -f1) - 1 ))p" "$0" |
                   sed 's/^# \?//'; exit 0 ;;
        *) echo "[rat-progress] unknown argument: $1 (try --help)" >&2; exit 2 ;;
    esac
done

# ---- the registry, from the runner itself: one list in the tree, not two ----
[ -f "$RUNNER" ] || { echo "[rat-progress] no $RUNNER — nothing defines the registry" >&2; exit 2; }
SUITES_LINE="$(grep -m1 '^SUITES=(' "$RUNNER")" ||
    { echo "[rat-progress] cannot find 'SUITES=(' in $RUNNER — refusing to guess the registry" >&2; exit 2; }
eval "$SUITES_LINE"
[ "${#SUITES[@]}" -gt 0 ] ||
    { echo "[rat-progress] parsed an EMPTY registry out of $RUNNER — refusing to report on it" >&2; exit 2; }

# ---- results: last row for a suite wins, exactly as the runner reads them back ----
declare -A ST SEC
recorded=0 wall=0
if [ -f "$RESULTS" ]; then
    while IFS=$'\t' read -r s st sec rc; do
        [ -n "${s:-}" ] || continue
        ST[$s]="${st:-?}"; SEC[$s]="${sec:-0}"
    done < "$RESULTS"
fi

# ---- one bucket per state; a suite the registry names but nothing ran is NOT-RUN ----
declare -A BUCKET
ORDER=(PASS FAIL TIMEOUT DIED NO-VERDICT MISSING NOT-RUN)
for s in "${SUITES[@]}"; do
    st="${ST[$s]:-NOT-RUN}"
    case "$st" in PASS|FAIL|TIMEOUT|DIED|NO-VERDICT|MISSING|NOT-RUN) ;; *) st="OTHER:$st" ;; esac
    BUCKET[$st]="${BUCKET[$st]:-}${s} "
    if [ "$st" != NOT-RUN ]; then
        recorded=$((recorded + 1))
        case "${SEC[$s]}" in ''|*[!0-9]*) ;; *) wall=$((wall + SEC[$s])) ;; esac
    fi
done
# a state the runner grows later must not vanish from this report
for st in "${!BUCKET[@]}"; do
    case " ${ORDER[*]} " in *" $st "*) ;; *) ORDER+=("$st") ;; esac
done

# ---- suites on disk in no registry: they never run at all, so they are not green ----
UNREG=()
for f in scripts/*-test.sh; do
    b="$(basename "$f")"; b="${b%-test.sh}"
    case " ${SUITES[*]} " in *" $b "*) ;; *) UNREG+=("$b") ;; esac
done

total=${#SUITES[@]}
remaining=$((total - recorded))
pass=0; for _ in ${BUCKET[PASS]:-}; do pass=$((pass + 1)); done

echo
echo "=========== registry run: statement of account (${RESULTS}) ==========="
printf '  %d of %d registered suites recorded (%d%%); %d still owed\n' \
    "$recorded" "$total" "$(( total ? recorded * 100 / total : 0 ))" "$remaining"
for st in "${ORDER[@]}"; do
    names="${BUCKET[$st]:-}"; [ -n "$names" ] || continue
    n=0; for _ in $names; do n=$((n + 1)); done
    note=""
    case "$st" in
        PASS)       note="ran and won" ;;
        FAIL)       note="ran, judged itself, LOST" ;;
        TIMEOUT)    note="hung, killed — NOT a pass" ;;
        DIED)       note="died before its own verdict — NOT a pass" ;;
        NO-VERDICT) note="exit 0 but printed no verdict — checked nothing" ;;
        MISSING)    note="registered, no script on disk — never ran" ;;
        NOT-RUN)    note="registered, no result recorded — NOT a pass" ;;
    esac
    printf '  %-11s %4d   %s\n' "$st" "$n" "$note"
    # every red is named in full: the count is the headline, the names are the work.
    if [ "$st" != PASS ] && [ "$st" != NOT-RUN ]; then
        printf '                   %s\n' $names
    elif [ "$st" = NOT-RUN ] && [ "$NAMES" = 1 ]; then
        printf '                   %s\n' $names | head -n "$NOTRUN_LIST"
        [ "$n" -gt "$NOTRUN_LIST" ] && echo "                   ... and $((n - NOTRUN_LIST)) more"
    elif [ "$st" = PASS ] && [ "$NAMES" = 1 ]; then
        printf '                   %s\n' $names
    fi
done
if [ "${#UNREG[@]}" -gt 0 ]; then
    printf '  %-11s %4d   on disk, in no registry — never runs at all\n' UNREGISTERED "${#UNREG[@]}"
    printf '                   %s-test.sh\n' "${UNREG[@]}"
fi

# ---- the rate, and what the remainder costs at it ----
# +1s per suite: the runner kills stray QEMU and sleeps 1 between suites, and that
# second is real wall-clock the recorded per-suite seconds do not contain.
echo "  ---------------------------------------------------------------"
if [ "$recorded" -gt 0 ]; then
    mean=$(( (wall + recorded) / recorded ))
    printf '  spent   %dh %02dm over %d suites (mean %ds/suite, boot included)\n' \
        "$(( (wall + recorded) / 3600 ))" "$(( ((wall + recorded) % 3600) / 60 ))" "$recorded" "$mean"
    if [ "$remaining" -gt 0 ]; then
        eta=$((mean * remaining))
        printf '  owed    %dh %02dm for the remaining %d, at tonight'\''s rate\n' \
            "$((eta / 3600))" "$(( (eta % 3600) / 60 ))" "$remaining"
        echo "  next    scripts/run-all-tests.sh --resume     # picks up where this stopped"
    fi
else
    echo "  nothing recorded yet: scripts/run-all-tests.sh --resume"
fi
echo "======================================================================"

# Green means the whole registry was walked and every step of it won. Anything else
# — including a run still in progress — is not green, and says so in its exit code.
[ "$remaining" -eq 0 ] && [ "$pass" -eq "$total" ] && [ "${#UNREG[@]}" -eq 0 ]
