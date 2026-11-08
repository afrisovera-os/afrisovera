#!/usr/bin/env bash
# uartrow-2914.sh -- does the uart-rx-stress SUITE ROW carry the 2.905 counters?
#
# 2.905 (g1) named the closer for the UART-site gap:
#   "uart-rx-stress's row now carries the two counters that NAME the site if it
#    reds again"
# This probe GRADES that claim against a real walk's evidence dir, instead of
# re-asserting it. It reads only; it never touches a running walk.
#
# Exit codes (disjoint, one per shape):
#   0  ROW-CARRIES     : uart-rx-stress evidence has a LINE with rx-drops AND rx-overruns
#  10  ROW-BLIND       : uart-rx-stress evidence has no such LINE, while some
#                        OTHER suite in the same dir does (instrument live, row blind)
#  11  INSTRUMENT-DEAD : no suite anywhere in the dir prints the counters
#                        (the 2.905 kernel is not what ran -- a different gap)
#  12  NO-EVIDENCE     : uart-rx-stress produced no log/capture in this dir
#   2  usage / missing dir
#
# Usage: bash scripts/uartrow-2914.sh [LOGDIR]   (default .rat-logs)
set -u

LOGDIR="${1:-.rat-logs}"
[ -d "$LOGDIR" ] || { echo "uartrow-2914: no such dir: $LOGDIR" >&2; exit 2; }

CTR='rx-drops'
CTR2='rx-overruns'

# The suite's evidence = everything it writes under LOGDIR.
mapfile -t STRESS < <(ls "$LOGDIR"/rat-uart-rx-stress.log "$LOGDIR"/uart-rx-stress.out 2>/dev/null)
if [ "${#STRESS[@]}" -eq 0 ]; then
    echo "UARTROW NO-EVIDENCE: uart-rx-stress wrote nothing under $LOGDIR"
    exit 12
fi

# TWO whole-dir grep -l passes, intersected -- NOT two greps per file. 2.783's
# lesson: on this host a per-file fork loop over ~1400 files costs minutes of
# pure fork and the probe times out before it ever answers. Two passes over the
# dir is two forks total, so the AND costs one extra fork, not 1400.
#
# 2.914 (g1') : this used to be ONE pass with `-e A -e B`, i.e. an OR -- a file
# printing rx-drops ALONE counted as a carrier. That biased the probe AGAINST
# declaring ROW-BLIND (the unflattering answer), so rc=10 was conservative and
# rc=0 was not earned. Then a carrier had to print BOTH counters.
#
# 2.917 (2.916 gap a) : that AND was per-FILE. Two counters landing in two
# UNRELATED lines of the same transcript is not the claim 2.905 made -- the
# claim is one irqstat READING, and 2.915 measured both counters on ONE line
# (`... rx-drops 0, rx-overruns 0`). A carrier is now a file with a LINE that
# carries both, in either order. Still ONE fork for the carrier test.
CARRY_LIST="$(grep -l -E "$CTR.*$CTR2|$CTR2.*$CTR" "$LOGDIR"/*.log "$LOGDIR"/*.out 2>/dev/null)"
# The two weaker shapes, for reporting only -- never counted as carriers.
L_DROPS="$(grep -l -e "$CTR"  "$LOGDIR"/*.log "$LOGDIR"/*.out 2>/dev/null)"
L_OVER="$(grep  -l -e "$CTR2" "$LOGDIR"/*.log "$LOGDIR"/*.out 2>/dev/null)"
# grep -l prints each file at most ONCE per pass, so a name seen twice across the
# two lists is exactly a file carrying both counters SOMEWHERE.
BOTH_ANY="$(printf '%s\n%s\n' "$L_DROPS" "$L_OVER" | grep -v '^$' | sort | uniq -d)"
# half = exactly one of the two counters anywhere in the file.
HALF_LIST="$(printf '%s\n%s\n' "$L_DROPS" "$L_OVER" | grep -v '^$' | sort | uniq -u)"
half=$(printf '%s' "$HALF_LIST" | grep -c . )
# split = both counters in the file but never on the same line (the shape the
# per-FILE AND used to score as a carrier).
SPLIT_LIST="$(printf '%s\n' "$BOTH_ANY" | grep -v '^$' | grep -F -x -v -f <(printf '%s\n' "$CARRY_LIST" | grep -v '^$') 2>/dev/null)"
split=$(printf '%s' "$SPLIT_LIST" | grep -c . )

stress_hits=0
others=0
other_name=""
while IFS= read -r f; do
    [ -n "$f" ] || continue
    case "$f" in
        *uart-rx-stress*) stress_hits=$((stress_hits + 1)); echo "  carries: $f" ;;
        *) others=$((others + 1)); [ -n "$other_name" ] || other_name="$f" ;;
    esac
done <<< "$CARRY_LIST"

echo "uartrow-2914: dir=$LOGDIR stress-files=${#STRESS[@]} stress-with-counters=$stress_hits other-files-with-counters=$others half-carriers=$half split-carriers=$split"

if [ "$stress_hits" -gt 0 ]; then
    echo "UARTROW ROW-CARRIES: uart-rx-stress evidence prints $CTR and $CTR2 on ONE line"
    exit 0
fi
if [ "$others" -gt 0 ]; then
    echo "UARTROW ROW-BLIND: the 2.905 counters are LIVE in this walk (e.g. $other_name)"
    echo "  but uart-rx-stress -- the suite 2.905 named as the site-namer -- does NOT print BOTH ON ONE LINE"
    echo "  (half-carriers in this dir: $half; split-carriers: $split; neither is a carrier)."
    echo "  A red uart-rx-stress row in this walk could NOT name the site."
    exit 10
fi
echo "UARTROW INSTRUMENT-DEAD: no file under $LOGDIR prints $CTR and $CTR2 on one line"
echo "  (weaker shapes present, and NOT carriers: half-carriers=$half split-carriers=$split)"
exit 11
