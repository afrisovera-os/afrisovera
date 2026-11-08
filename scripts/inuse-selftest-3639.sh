#!/usr/bin/env bash
# Phase 3.639 selftest - reach BOTH arms of every repaired predicate in
# scripts/inuse-control-3639.sh OFFLINE, before spending two real QEMU boots.
#
# Law 9 (2.766): "anchor fragment checks on the DELIMITER + reach both arms
# offline before spending the run."  3.637's control script was green-by-
# construction on a failed read; the only defence is showing the repaired
# predicate SAYING NO on the shapes that must be refused.
#
# The predicates below are EXTRACTED VERBATIM from inuse-control-3639.sh by
# sourcing nothing and copying nothing by hand: read_word is pulled out of the
# file with sed, so a divergence between this selftest and the instrument it
# grades cannot go unnoticed.
#
# Exit 0 SELFTEST-3639-GREEN / 20 SELFTEST-3639-RED / 10 PRECONDITION-UNMET
set -uo pipefail
cd "$(dirname "$0")/.."

SRC="scripts/inuse-control-3639.sh"
[ -s "$SRC" ] || { echo "PRECONDITION UNMET: $SRC missing"; exit 10; }

FAIL=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; FAIL=$((FAIL+1)); }

# ---- pull read_word out of the instrument itself, do not retype it ----------
RW_LINE="$(grep -n '^read_word()' "$SRC" | head -1 | cut -d: -f1)"
[ -n "$RW_LINE" ] || { echo "PRECONDITION UNMET: read_word not found in $SRC"; exit 10; }
eval "$(sed -n "${RW_LINE}p" "$SRC")"
echo "== the predicate under test, lifted from $SRC:$RW_LINE =="
sed -n "${RW_LINE}p" "$SRC" | sed 's/^/  /'

T="${TMPDIR:-/tmp}"
FX_IND="$T/st3639-indented.out"    # the PROBE's shape: '  | HOST-LISTENER=...'
FX_COL="$T/st3639-column0.out"     # the SUITE's shape: column 0
FX_NONE="$T/st3639-none.out"       # no word at all
trap 'rm -f "$FX_IND" "$FX_COL" "$FX_NONE"' EXIT

# ---- A: the two REAL streams, not invented ones -----------------------------
echo "== A: read_word on the two REAL streams from the 03:28Z run =="
for f in .inuse-3637.out .inuse-3637-trt-suite.out; do
    [ -s "$f" ] || { echo "PRECONDITION UNMET: fixture $f missing"; exit 10; }
done
W_PROBE="$(read_word .inuse-3637.out)"
W_SUITE="$(read_word .inuse-3637-trt-suite.out)"
case "$W_PROBE" in
    HOST-LISTENER=foreign*) ok "1 INDENTED stream (probe stdout) reads: $W_PROBE" ;;
    *) bad "1 INDENTED stream read '$W_PROBE' - the A-repair does not work on the real file" ;;
esac
case "$W_SUITE" in
    HOST-LISTENER=foreign*) ok "2 COLUMN-0 stream (raw suite transcript) reads: $W_SUITE" ;;
    *) bad "2 COLUMN-0 stream read '$W_SUITE' - the same expression must serve both streams" ;;
esac
[ "$W_PROBE" = "$W_SUITE" ] \
    && ok "3 both streams yield the IDENTICAL word - the read no longer depends on which stream carried it" \
    || bad "3 the two streams disagree: '$W_PROBE' vs '$W_SUITE'"
OLD="$(grep -m1 '^HOST-LISTENER' .inuse-3637.out 2>/dev/null || true)"
[ -z "$OLD" ] \
    && ok "4 the ORPHAN expression still reads EMPTY off the same file - defect A reproduces here too" \
    || bad "4 the orphan expression read '$OLD' - defect A does not reproduce"

# ---- B: the shapes that must be REFUSED --------------------------------------
echo "== B: read_word must refuse what is not the word =="
printf 'some prose about HOST-LISTENER=own in the middle of a line\n' > "$FX_NONE"
W_NONE="$(read_word "$FX_NONE")"
[ -z "$W_NONE" ] \
    && ok "5 a mid-line mention is NOT read as the word (empty)" \
    || bad "5 a mid-line mention was read as '$W_NONE'"
W_MISS="$(read_word /nonexistent-3639)"
[ -z "$W_MISS" ] \
    && ok "6 a MISSING file reads empty (and arm 3 of the instrument refuses empty)" \
    || bad "6 a missing file read '$W_MISS'"
printf '  | HOST-LISTENER=own (this suite bound 127.0.0.1:80)\n' > "$FX_IND"
printf 'HOST-LISTENER=own (this suite bound 127.0.0.1:80)\n'     > "$FX_COL"
[ "$(read_word "$FX_IND")" = "$(read_word "$FX_COL")" ] && [ -n "$(read_word "$FX_COL")" ] \
    && ok "7 the OWN word reads identically indented and at column 0: $(read_word "$FX_COL")" \
    || bad "7 the OWN word does not survive both shapes"

# ---- C: the arm-3 flip predicate, ALL FOUR outcomes reached ------------------
# copied in shape from the instrument; each case is exercised, none is argued.
flip_verdict() {
    local C="$1" TT="$2"
    if [ -z "$C" ] || [ -z "$TT" ]; then echo EMPTY; return; fi
    if [ "$C" = "$TT" ]; then echo SAME; return; fi
    case "$C:$TT" in
        HOST-LISTENER=own*:HOST-LISTENER=foreign*) echo FLIPPED ;;
        *) echo WRONG-PAIR ;;
    esac
}
echo "== C: the flip predicate, all four outcomes =="
V=$(flip_verdict "HOST-LISTENER=own (x)" "");            [ "$V" = EMPTY ]      && ok "8 empty treatment word -> $V (the orphan said FLIPPED here)"   || bad "8 empty treatment word -> $V"
V=$(flip_verdict "" "HOST-LISTENER=foreign (x)");        [ "$V" = EMPTY ]      && ok "9 empty control word -> $V"                                     || bad "9 empty control word -> $V"
V=$(flip_verdict "HOST-LISTENER=own (x)" "HOST-LISTENER=own (x)"); [ "$V" = SAME ] && ok "10 identical words -> $V"                                    || bad "10 identical words -> $V"
V=$(flip_verdict "HOST-LISTENER=foreign (x)" "HOST-LISTENER=own (x)"); [ "$V" = WRONG-PAIR ] && ok "11 BACKWARDS pair (foreign->own) -> $V"            || bad "11 backwards pair -> $V"
V=$(flip_verdict "HOST-LISTENER=own (this suite bound 127.0.0.1:80)" "$W_PROBE"); [ "$V" = FLIPPED ] && ok "12 own + the REAL foreign word -> $V"       || bad "12 the genuine pair -> $V"

# the ORPHAN predicate, for the differential: it says FLIPPED on arm 8's input
orphan_flip() { local C="$1" TT="$2"; if [ -n "$C" ] && [ "$C" != "$TT" ]; then echo FLIPPED; else echo NO; fi; }
V=$(orphan_flip "HOST-LISTENER=own (x)" "")
[ "$V" = FLIPPED ] \
    && ok "13 DIFFERENTIAL: the orphan predicate says $V on the SAME empty-read input the repair refuses" \
    || bad "13 the orphan predicate said $V - defect B does not reproduce"

# ---- D: the walk detector, both arms ----------------------------------------
echo "== D: the C-repair detector, both arms, measured =="
PSF="$(ps -f 2>/dev/null)"
SELFN=$(printf '%s\n' "$PSF" | grep -c "[i]nuse-selftest-3639"); SELFN=${SELFN:-0}
[ "$SELFN" -ge 1 ] \
    && ok "14 ps -f SEES this running script's own cmdline ($SELFN) - the detector can see bash script names" \
    || bad "14 ps -f could not see this script ($SELFN) - the detector is blind"
SELFW=$(ps -W 2>/dev/null | grep -c "[i]nuse-selftest-3639"); SELFW=${SELFW:-0}
[ "$SELFW" -eq 0 ] \
    && ok "15 DIFFERENTIAL: ps -W sees the SAME live script $SELFW times - defect C reproduces on a script proven live" \
    || bad "15 ps -W saw this script $SELFW times - defect C does not reproduce"
ABSENT=$(printf '%s\n' "$PSF" | grep -c "[n]o-such-script-3639-xyz"); ABSENT=${ABSENT:-0}
[ "$ABSENT" -eq 0 ] \
    && ok "16 ...and the same detector returns 0 for a name that genuinely is not running" \
    || bad "16 detector returned $ABSENT for an absent name"

echo "-- $((16-FAIL)) passed, $FAIL failed --"
[ "$FAIL" -eq 0 ] && { echo "SELFTEST-3639-GREEN"; exit 0; }
echo "SELFTEST-3639-RED"; exit 20
