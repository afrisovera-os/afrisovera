#!/usr/bin/env bash
# 3.357 -- falsify-first arm for the 3.349 honest gap:
#   "capture_writable() must TEST all paths before REMOVING any."
#
# At 3.349 a colliding probe run refused RIG-BUSY on c.wav with ZERO boots -- but by then
# it had ALREADY rm -f'd the incumbent's COMPLETED a.wav (192044 bytes), destroying a
# capture it never owned. The refusal was right; the collateral damage was not.
#
# This does NOT transcribe the predicate (the 3.354 lesson says transcription is a last
# resort). It EXTRACTS the two live functions out of scripts/speaktext-probe-3263.sh with
# sed and evals them, so it always grades the code that is actually on disk. It boots
# NOTHING: the busy path is simulated by a NON-EMPTY DIRECTORY, on which `rm -f` fails and
# `-e` stays true -- the exact shape of a path whose handle cannot be cleared.
#
# EXIT 10 = HAZARD-REPRODUCED (refusal destroyed the untouched sibling)  <- the failing arm
# EXIT  0 = PRESERVED + no regression on the all-clear path
# EXIT  2 = the arm itself could not run (extraction failed) -- graded neither way
set -u

PROBE="${PROBE:-scripts/speaktext-probe-3263.sh}"
T="$(mktemp -d 2>/dev/null || echo ".pf3357.$$")"
mkdir -p "$T" || exit 2
trap 'rm -rf "$T"' EXIT

[ -f "$PROBE" ] || { echo "PREFLIGHT-FALSIFY: no such probe: $PROBE"; exit 2; }

# ---- extract the live functions (never a copy of them) -------------------------------
SRC="$T/extracted.sh"
sed -n '/^capture_writable() {/,/^}/p'   "$PROBE" >  "$SRC"
sed -n '/^preflight_captures() {/,/^}/p' "$PROBE" >> "$SRC"
grep -q '^capture_writable() {'   "$SRC" || { echo "PREFLIGHT-FALSIFY: capture_writable not extracted";   exit 2; }
grep -q '^preflight_captures() {' "$SRC" || { echo "PREFLIGHT-FALSIFY: preflight_captures not extracted"; exit 2; }
echo "[arm] extracted $(wc -l < "$SRC") lines of LIVE code from $PROBE"
# shellcheck disable=SC1090
. "$SRC" || { echo "PREFLIGHT-FALSIFY: extracted code did not source"; exit 2; }

INCUMBENT="the completed capture of a run that already finished"

setup_busy() {   # a = a finished capture; b = free; c = a path that cannot be cleared
    rm -rf "$T/a.wav" "$T/b.wav" "$T/c.wav"
    printf '%s' "$INCUMBENT" > "$T/a.wav"
    mkdir -p "$T/c.wav" && printf 'held' > "$T/c.wav/handle"
}
setup_clear() {
    rm -rf "$T/a.wav" "$T/b.wav" "$T/c.wav"
    printf '%s' "$INCUMBENT" > "$T/a.wav"
}

pass=0; fail=0
ck() { if [ "$2" = "$3" ]; then echo "  ok   $1 ($3)"; pass=$((pass+1)); else echo "  FAIL $1 (want $2, got $3)"; fail=$((fail+1)); fi; }

# ---- ARM 1: a busy path must NOT cost the untouched sibling its bytes ------------------
setup_busy
out1="$(preflight_captures "$T/a.wav" "$T/b.wav" "$T/c.wav" 2>&1)"; rc1=$?
echo "[arm 1] refusal on a busy third path (rc=$rc1)"
echo "$out1" | sed 's/^/    | /'
ck "arm1 refuses"            1 "$rc1"
if [ -f "$T/a.wav" ] && [ "$(cat "$T/a.wav")" = "$INCUMBENT" ]; then a1=preserved; else a1=destroyed; fi
echo "[arm 1] sibling a.wav -> $a1"

# ---- ARM 2: the all-clear path still clears (no regression) ---------------------------
setup_clear
out2="$(preflight_captures "$T/a.wav" "$T/b.wav" "$T/c.wav" 2>&1)"; rc2=$?
echo "[arm 2] all paths free (rc=$rc2)"
echo "$out2" | sed 's/^/    | /'
ck "arm2 accepts"            0 "$rc2"
[ -e "$T/a.wav" ] && a2=left || a2=cleared
ck "arm2 clears a.wav"       cleared "$a2"
[ -e "$T/b.wav" ] && b2=left || b2=cleared
ck "arm2 leaves no b.wav"    cleared "$b2"

# ---- ARM 3: a truly unwritable path is still caught (the refusal itself survives) ------
setup_busy
out3="$(capture_writable "$T/c.wav" 2>&1)"; rc3=$?
echo "[arm 3] the busy path alone (rc=$rc3): $out3"
ck "arm3 refuses"            1 "$rc3"
case "$out3" in *"$T/c.wav"*) n3=names ;; *) n3=silent ;; esac
ck "arm3 names the path"     names "$n3"

echo
echo "arms reached: $((pass+fail)) | pass=$pass fail=$fail | sibling=$a1"
if [ "$fail" -ne 0 ]; then
    echo "PREFLIGHT-FALSIFY INCONCLUSIVE: an arm the hazard does not depend on is red"
    exit 2
fi
if [ "$a1" = destroyed ]; then
    echo "PREFLIGHT-FALSIFY: HAZARD-REPRODUCED -- the refusal deleted the incumbent's finished capture"
    exit 10
fi
echo "PREFLIGHT-FALSIFY GREEN: refusal preserved the sibling, all-clear path unchanged"
exit 0
