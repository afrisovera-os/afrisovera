#!/usr/bin/env bash
# longspeak-probe-3369.sh -- the FAILING ARM for design-3.367 (B), the streaming
# rewrite of voice::say().
#
# THE PROPERTY UNDER TEST: a text longer than the ~1.09 s utterance buffer is
# said IN FULL. Today it is not -- voice.rs:26 MAXLEN = 48000 samples and
# voice.rs:209 `if n >= MAXLEN { break; }` cut the sentence off (finding-3.365),
# so the captured PCM is EXACTLY MAXLEN * 4 = 192000 bytes no matter how long
# the text is. That equality IS the ceiling; it is not a property of the text.
#
# EXPECTED TODAY (unrewired tree): exit 10 UTTERANCE-CEILING-HELD.
# EXPECTED AFTER the streaming rewrite, from this SAME UNEDITED script:
#          exit 0  UTTERANCE-COMPLETE (pcm > 192000).
#
# DELIVERY ROUTE (measured, not assumed): the `say` shell command
# (shell.rs:1291) passes `rest.as_bytes()` straight to voice::say with NO cap in
# that function, so an arbitrarily long text needs NO kernel rebuild and NO
# second baked module -- unlike the WASM speak() path, which 3.261 measured has
# no runtime delivery route. This probe therefore CALLS scripts/audio-shot.sh
# UNEDITED (2.765 rule 5) with one `say <text>` command.
#
# GRADED: the PCM byte count of the capture, against the ceiling constant, PLUS
# an INDEPENDENT second derivation -- the guest's own `[asili voice] TRUNCATED:`
# line (3.366). The two must AGREE; a disagreement REFUSES a verdict rather than
# picking the flattering one.
#
# NOT GRADED, deliberately:
#   * the "[asili voice] (would speak aloud)" line -- that line fooled docs/14 at
#     3.254 and says nothing about audio;
#   * a capture SHORTER than the ceiling -- that is 3.350's still-unnamed
#     truncation class, NOT this property. It refuses (RIG-TRUNCATED, exit 1).
set -uo pipefail

CEILING=192000            # MAXLEN(48000) * 2 ch * 2 B -- voice.rs:26
HDR=44                    # bare RIFF header written by QEMU's wav audiodev
LOGDIR="${LOGDIR:-.longspeak-3369-logs}"
WAVF="${WAVF:-.longspeak-3369.wav}"

# ~124 chars, far past the ~10 that fit under the ceiling (3.365's per-char
# arithmetic). No quotes/backslashes: the driver sends this line verbatim.
TEXT="THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG WHILE THE KETTLE BOILS AND THE RADIO PLAYS A SLOW SONG ABOUT THE LONG ROAD HOME."

# ---------------------------------------------------------------- grader ----
# pcm_bytes -> verdict word. Pure function of ONE number, so it is selftestable
# offline with no boot.
grade_pcm() {
    local n="$1"
    if [ -z "$n" ] || [ "$n" -lt 0 ] 2>/dev/null; then echo "BAD"; return; fi
    if   [ "$n" -lt "$CEILING" ]; then echo "SHORT"
    elif [ "$n" -eq "$CEILING" ]; then echo "CEILING"
    else                               echo "COMPLETE"
    fi
}

selftest() {
    local fails=0 got
    # arm 1: below the ceiling is the 3.350 class, NOT this property
    got=$(grade_pcm 40960);  [ "$got" = "SHORT" ]    || { echo "[selftest] arm1 want SHORT got $got"; fails=1; }
    # arm 2: exactly the ceiling is the defect this probe is here to reproduce
    got=$(grade_pcm 192000); [ "$got" = "CEILING" ]  || { echo "[selftest] arm2 want CEILING got $got"; fails=1; }
    # arm 3: one period past it is the cure -- reachable, so the probe can pass
    got=$(grade_pcm 196096); [ "$got" = "COMPLETE" ] || { echo "[selftest] arm3 want COMPLETE got $got"; fails=1; }
    # arm 4: a single byte past it also passes (no fudge band hiding a near-miss)
    got=$(grade_pcm 192001); [ "$got" = "COMPLETE" ] || { echo "[selftest] arm4 want COMPLETE got $got"; fails=1; }
    # arm 5: an absent/garbage size never grades
    got=$(grade_pcm "");     [ "$got" = "BAD" ]      || { echo "[selftest] arm5 want BAD got $got"; fails=1; }
    if [ "$fails" -ne 0 ]; then echo "SELFTEST-FAILED -- grader is broken; no boot spent"; exit 1; fi
    echo "[probe] grader selftest 5/5 arms reached, offline, before any boot"
}

# ------------------------------------------------------------- preflight ----
# 3.349/3.350: the label must NOT carry the pattern, or ps matches its own line.
# MEASURED here, and it is why THIS probe does not ps for ITSELF: `ps -ef` shows
# four lines carrying this script's name for a single run (the outer `bash -c`
# whose eval string quotes it, `timeout`, the script, and the `$( )` fork), and
# ancestry cannot filter them -- cygwin reports PPID=1 for all of them. A
# self-count would refuse every run. So a concurrent INSTANCE is detected
# directly, by an atomic lock; ps is kept only for the processes that have no
# self-match problem.
LOCKDIR="${LOCKDIR:-.longspeak-3369.lock}"
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "RIG-BUSY: $LOCKDIR exists -- another longspeak probe holds it."
    echo "          (If no probe is running, this is a stale lock: rmdir $LOCKDIR)"
    exit 1
fi
trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT

# 3.373: the `[r]un-all-tests` and `qemu` ps arms are GONE. They were UNSATISFIABLE
# inside the registry runner -- MEASURED at 3.372: `--only longspeak` through the real
# runner gave `longspeak FAIL 43` because the run-all-tests count is 2 BY CONSTRUCTION
# there (the runner itself plus its per-suite `timeout`/subshell), so the probe refused
# RIG-BUSY with zero boots and all four of suite 688's grading arms went red. A suite
# that cannot run inside a walk is not a suite. The form below is the SIBLING's, not an
# invention: `scripts/speaktext-probe-3263.sh` (suite 687, which PASSED inside walk 3362)
# has no run-all-tests/qemu ps guard at all -- it guards on CAPTURE-PATH WRITABILITY plus
# its own atomic lock, and both of those ARE satisfiable inside a walk. The lock above is
# what actually excludes a concurrent instance of THIS probe; ps never could.
pf() { ps -ef 2>/dev/null | grep -c "$1"; }
n_spk=$(pf "[s]peaktext-probe")
echo "[probe] preflight: lock=held speaktext=$n_spk"
if [ "$n_spk" -gt 0 ]; then
    echo "RIG-BUSY: a speaktext probe is live; refusing before touching anything"
    exit 1
fi

# capture_writable <path> -- NON-DESTRUCTIVE (3.349's lesson, taken from the sibling):
# it holds any incumbent aside, proves the path can be cleared and re-created, then PUTS
# IT BACK. Removal is the CALLER's second phase, reached only when every path passed, so
# a refusal destroys nothing it does not own.
capture_writable() { # <path>
    local p="$1" held=""
    if [ -e "$p" ] && [ ! -f "$p" ]; then echo "not-a-file:$p"; return 1; fi
    if [ -e "$p" ]; then
        held="$p.pfhold.$$"
        mv -f "$p" "$held" 2>/dev/null
        if [ -e "$p" ]; then echo "still-present-after-rm:$p"; return 1; fi
    fi
    if ! : >"$p" 2>/dev/null; then
        echo "cannot-create:$p"; [ -n "$held" ] && mv -f "$held" "$p" 2>/dev/null; return 1
    fi
    rm -f "$p" 2>/dev/null
    if [ -e "$p" ]; then
        echo "cannot-remove-fresh:$p"; [ -n "$held" ] && rm -f "$held" 2>/dev/null; return 1
    fi
    [ -n "$held" ] && mv -f "$held" "$p" 2>/dev/null
    return 0
}
preflight_captures() { # <path>...
    local p why bad=0
    for p in "$@"; do
        if why="$(capture_writable "$p")"; then echo "[probe] preflight $p -> writable"
        else echo "[probe] preflight $p -> BUSY ($why)"; bad=1; fi
    done
    [ "$bad" -ne 0 ] && return 1
    for p in "$@"; do rm -f "$p" 2>/dev/null; done   # PHASE 2: the only destruction
    return 0
}

# Both directions of the new guard, OFFLINE, before any boot. The line below deliberately
# does NOT say "grader selftest N/N arms reached" -- suite 688 anchors on that exact text
# for the pcm->verdict grader, and two selftests must not be gradeable as one.
preflight_selftest() {
    local d rc=0 n=0
    d="$(mktemp -d)" || { echo "SELFTEST-FAILED: no tmpdir"; exit 1; }
    # arm 1: a free path is writable
    capture_writable "$d/free.wav" >/dev/null && n=$((n+1)) || rc=1
    # arm 2: an existing capture SURVIVES the test (non-destructive)
    printf 'incumbent' >"$d/held.wav"
    capture_writable "$d/held.wav" >/dev/null
    [ "$(cat "$d/held.wav" 2>/dev/null)" = "incumbent" ] && n=$((n+1)) || rc=1
    # arm 3: a directory on a capture path is refused and NOT renamed away
    mkdir -p "$d/dir.wav"
    capture_writable "$d/dir.wav" >/dev/null || { [ -d "$d/dir.wav" ] && n=$((n+1)); }
    [ "$n" -eq 3 ] || rc=1
    rm -rf "$d"
    echo "[probe] preflight selftest $n/3 arms reached"
    [ "$rc" -eq 0 ] || { echo "SELFTEST-FAILED: the capture preflight is broken"; exit 1; }
}
preflight_selftest

selftest

if [ "${#TEXT}" -lt 60 ]; then
    echo "RIG-BROKEN: TEXT is ${#TEXT} chars, not long enough to exceed the ceiling"; exit 1
fi

mkdir -p "$LOGDIR"
preflight_captures "$WAVF" || { echo "RIG-BUSY: capture path is not clearable"; exit 1; }

# ------------------------------------------------------------------ boot ----
echo "[probe] one boot: say <${#TEXT} chars>"
WAV="$WAVF" ./scripts/audio-shot.sh "$LOGDIR/long.png" "say $TEXT" >"$LOGDIR/long.serial" 2>&1
RIGRC=$?
echo "[probe] rig rc=$RIGRC"
if [ ! -s "$WAVF" ]; then
    echo "RIG-BROKEN: no capture at $WAVF"; tail -5 "$LOGDIR/long.serial"; exit 1
fi

SZ=$(wc -c <"$WAVF" | tr -d ' ')
PCM=$(( SZ - HDR ))
V=$(grade_pcm "$PCM")
echo "[probe] wav bytes=$SZ pcm=$PCM ceiling=$CEILING verdict=$V"

# Second, INDEPENDENT derivation off the guest's own line (3.366), anchored on
# the marker so a restatement elsewhere cannot grade the same thing twice.
if grep -q "asili voice] TRUNCATED:" "$LOGDIR/long.serial"; then GUEST=TRUNCATED; else GUEST=WHOLE; fi
grep -o "asili voice] TRUNCATED:.*" "$LOGDIR/long.serial" | head -1
echo "[probe] guest says: $GUEST"

case "$V" in
  SHORT)
    echo "RIG-TRUNCATED: pcm=$PCM is BELOW the ceiling -- that is 3.350's unnamed"
    echo "               capture class, not this property. NO verdict taken."
    exit 1 ;;
  BAD)
    echo "RIG-BROKEN: ungradeable capture size"; exit 1 ;;
  CEILING)
    if [ "$GUEST" != "TRUNCATED" ]; then
        echo "DISAGREEMENT: pcm sits exactly on the ceiling but the guest printed no"
        echo "              TRUNCATED line. Two derivations disagree; NO verdict taken."
        exit 1
    fi
    echo "UTTERANCE-CEILING-HELD: say() of ${#TEXT} chars produced exactly $PCM PCM bytes"
    echo "  (= MAXLEN * 4) and the guest itself reported TRUNCATED. The sentence was cut."
    exit 10 ;;
  COMPLETE)
    if [ "$GUEST" = "TRUNCATED" ]; then
        echo "DISAGREEMENT: pcm exceeds the ceiling but the guest still printed TRUNCATED."
        echo "              Two derivations disagree; NO verdict taken."
        exit 1
    fi
    echo "UTTERANCE-COMPLETE: say() of ${#TEXT} chars produced $PCM PCM bytes, past the"
    echo "  $CEILING-byte ceiling, with no TRUNCATED line. The whole sentence was said."
    exit 0 ;;
esac
