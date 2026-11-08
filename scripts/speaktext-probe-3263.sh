#!/usr/bin/env bash
# 3.263 falsify-first probe, PRE-REGISTERED at 3.260
# (docs/finding-3.260-wasm-speak-wired-to-the-wrong-half.md).
#
# Claim under test: a Nia app's speak(text) is TEXT-INDEPENDENT BY CONSTRUCTION —
# HOST_SPEAK (nia.rs) calls virtio::sound_play_tone(), which takes NO argument, while
# the real text-dependent formant synthesizer (voice::say) sits one module away with
# two other callers. 3.256 wired speak() to the PLACEHOLDER half.
#
# THIS PROBE GRADES THE PCM PAYLOADS OF TWO BOOTS AGAINST EACH OTHER.
# Deliberately NOT graded (3.260's pre-commitment):
#   * the "[asili voice] (would speak aloud)" serial line — that line is unchanged on
#     BOTH sides and is exactly the evidence that fooled docs/14 at 3.254;
#   * the PCM byte COUNT alone — a tone and speech both fill the same capture window;
#   * suite 686 `audio` passing (single text) and the say/EL0 paths — that conflation
#     IS the finding.
#
# THE CONTROL IS IN-RUN, NOT ACROSS TIME (suite 686's stated gap): notify.wasm is booted
# TWICE. If two boots of the SAME text already differ, the run-to-run noise floor swamps
# the signal and NO verdict is taken. Without that arm, "the two texts differ" would be
# unfalsifiable.
#
#   exit 10 SPEAK-TEXT-INDEPENDENT = expected on this UNREWIRED tree (finding reproduces)
#   exit  0 SPEAK-TEXT-DEPENDENT   = expected only AFTER HOST_SPEAK -> crate::voice::say
#   exit  1 RIG-BROKEN / RIG-NOISY = no verdict (boot failed, or control boots disagree)
#   exit  4 RIG-BUSY               = no verdict, NO BOOTS TAKEN — a capture file could not
#                                    be made writable before the run (3.269 run 1: a QEMU
#                                    from the invocation 55 s earlier still held boot b's
#                                    .wav, so `rm` said "Device or resource busy" and the
#                                    run limped on to a confusing RIG-BROKEN three boots
#                                    later). This is a REFUSAL, never a retry loop.
#   exit  5 RIG-UNSETTLED          = no verdict — a capture was STILL CHANGING SIZE when the
#                                    deadline expired, so nothing was graded (3.272's NEXT:
#                                    the POST-flight read barrier). Also a REFUSAL, never a
#                                    retry loop and never a re-boot.
#
# Flipping 10 -> 0 buys FORMANT speech, NOT Piper — the neural model stays unclaimed.
set -uo pipefail

WAV_A="${WAV_A:-.speaktext-3263-a.wav}"   # notify.wasm  (text 1)
WAV_C="${WAV_C:-.speaktext-3263-c.wav}"   # notify.wasm  (text 1 again — the control)
WAV_B="${WAV_B:-.speaktext-3263-b.wav}"   # notify2.wasm (text 2)
LOGDIR="${LOGDIR:-.speaktext-3263-logs}"

# ---------------------------------------------------------------- the grader
# Compares the PCM payload (past the 44-byte RIFF header) of two WAVs.
# Prints: BAD <why> | SAME n=<bytes> md5=<hex> | DIFF n1=<> n2=<> firstdiff=<idx|len>
cmp_pcm() {
python3 - "$1" "$2" <<'PY'
import sys, os, hashlib
def payload(p):
    if not os.path.exists(p):
        return None, "no-file:%s" % os.path.basename(p)
    b = open(p, 'rb').read()
    if len(b) <= 44:
        return None, "header-only:%s bytes=%d" % (os.path.basename(p), len(b))
    pcm = b[44:]
    if len(set(pcm)) <= 1:
        return None, "silent:%s pcm=%d distinct=%d" % (os.path.basename(p), len(pcm), len(set(pcm)))
    return pcm, None
p1, e1 = payload(sys.argv[1])
p2, e2 = payload(sys.argv[2])
if e1 or e2:
    print("BAD %s" % (e1 or e2)); sys.exit(0)
if p1 == p2:
    print("SAME n=%d md5=%s" % (len(p1), hashlib.md5(p1).hexdigest())); sys.exit(0)
fd = "len"
if len(p1) == len(p2):
    fd = str(next(i for i in range(len(p1)) if p1[i] != p2[i]))
print("DIFF n1=%d n2=%d firstdiff=%s md5a=%s md5b=%s"
      % (len(p1), len(p2), fd, hashlib.md5(p1).hexdigest()[:8], hashlib.md5(p2).hexdigest()[:8]))
PY
}

# ------------------------------------------------------- the pre-flight (3.269's NEXT)
# One capture path. Prints a reason and returns 1 when the path cannot be cleared and
# re-created; prints nothing and returns 0 when it can. Deliberately does NOT look for
# stray QEMU processes: what breaks the run is the HANDLE, and the handle is exactly what
# a delete+create tests. No sleep, no retry — a busy path is a refusal.
#
# 3.357: this is now a TEST ONLY -- it holds the incumbent aside, proves the path can be
# cleared and re-created, then PUTS IT BACK. At 3.349 the old form rm -f'd every path it
# looked at and only refused on the one whose rm failed, so a colliding run destroyed an
# already-finished capture (192044 bytes) it never owned. Removal is now the CALLER's
# second phase, reached only when every path has passed.
capture_writable() { # <path>  -- non-destructive; leaves $p exactly as it found it
    local p="$1" held=""
    # A directory (or anything not a regular file) sitting on a capture path is unusable
    # and must never be renamed out of the way.
    if [ -e "$p" ] && [ ! -f "$p" ]; then echo "not-a-file:$p"; return 1; fi
    if [ -e "$p" ]; then
        held="$p.pfhold.$$"
        mv -f "$p" "$held" 2>/dev/null
        # A path whose handle is still held cannot be renamed either: $p survives the mv.
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
        if why="$(capture_writable "$p")"; then
            echo "[preflight] $p -> writable"
        else
            echo "[preflight] $p -> BUSY ($why)"; bad=1
        fi
    done
    # PHASE 2 -- the only place a capture is actually destroyed, and only once EVERY path
    # has been proven clearable. A refusal above returns having deleted nothing.
    [ "$bad" -ne 0 ] && return 1
    for p in "$@"; do rm -f "$p" 2>/dev/null; done
    return "$bad"
}

# ------------------------------------------- the post-flight read barrier (3.272's NEXT)
# 3.267 and 3.271 measured boot b truncating on an EXACT 4096 boundary in 4 of 18 boots
# (53248 = 13x4096, against a complete 192000 which is NOT a page multiple). 3.272 reordered
# the boots a,c,b -> a,b,c and got 9 quiet boots, but P(0 in 9 | 0.222/boot) = 0.10, so the
# reorder is NOT a cure and NEITHER model was confirmed. The two surviving models agree on
# exactly one thing: the capture that is READ SOONEST after its writer exits is the one that
# can be read mid-flush. So no capture is graded until its size has STOPPED CHANGING.
#
# This is a BARRIER, not a sleep and not a retry: it samples, it waits for stationarity, and
# on a deadline it REFUSES (exit 5) rather than looping or grading what it has. Correct under
# model M (last-slot read-before-flush); HARMLESS under model N (a writer that closes the
# file short) — under N an already-closed short capture settles immediately and the barrier
# neither hides it nor cures it. HONEST GAP, stated not fixed: stationarity cannot tell a
# flushed-complete capture from a closed-short one; that is boot b's own control's job
# (3.268), not this barrier's.
SETTLE_DT="${SETTLE_DT:-0.25}"    # seconds between samples
SETTLE_K="${SETTLE_K:-3}"         # consecutive EQUAL non-zero samples that count as stationary
SETTLE_MAX="${SETTLE_MAX:-40}"    # hard sample cap => ~10 s at the default dt, then REFUSE
settle_capture() { # <path> -> echoes "<bytes> <samples>"; rc 0 stationary, rc 1 refused
    local p="$1" size prev="" run=0 n=0
    while [ "$n" -lt "$SETTLE_MAX" ]; do
        # -f FIRST: `wc -c <missing` fails in the SHELL, so `2>/dev/null` on wc does not
        # suppress it and an absent capture printed six scary lines per refusal arm.
        if [ -f "$p" ]; then size="$(wc -c <"$p")"; else size=0; fi
        size="${size// /}"
        if [ "$size" = "$prev" ] && [ "$size" -gt 0 ] 2>/dev/null; then
            run=$((run+1))
            if [ "$run" -ge "$SETTLE_K" ]; then echo "$size $n"; return 0; fi
        else
            run=1
        fi
        prev="$size"; n=$((n+1))
        sleep "$SETTLE_DT"
    done
    if [ -f "$p" ]; then size="$(wc -c <"$p")"; else size=0; fi
    echo "${size// /} $n"; return 1
}

# ------------------- barrier selftest (Law 9: BOTH directions, offline, BEFORE any boot)
# A zero-length capture must NEVER read as stationary — "it stopped changing at 0 bytes" is a
# broken rig, not a settled one, and grading it would be the flattering direction.
sb_dir="$(mktemp -d)"
sb_fail=0
sb_arm() { # <label> <ok|refuse> <path> [expected-bytes]
    local out rc
    out="$(settle_capture "$3")"; rc=$?
    if [ "$rc" -eq 0 ]; then got=ok; else got=refuse; fi
    if [ "$got" != "$2" ]; then
        echo "[barrier-selftest] $1 -> $got ($out)  EXPECTED $2  FAIL"; sb_fail=1; return
    fi
    if [ -n "${4:-}" ] && [ "${out%% *}" != "$4" ]; then
        echo "[barrier-selftest] $1 -> $got bytes=${out%% *}  EXPECTED bytes=$4  FAIL"; sb_fail=1; return
    fi
    echo "[barrier-selftest] $1 -> $got ($out)  OK"
}
head -c 4096 /dev/urandom > "$sb_dir/static.bin"
SETTLE_MAX=12 sb_arm "already stationary"     ok     "$sb_dir/static.bin" 4096
: > "$sb_dir/empty.bin"
SETTLE_MAX=6  sb_arm "zero-length capture"    refuse "$sb_dir/empty.bin" 0
SETTLE_MAX=6  sb_arm "absent capture"         refuse "$sb_dir/absent.bin" 0
# grows for ~1.5 s then STOPS: the barrier must wait it out and report the FINAL size, which
# is the whole point — a naive read takes the first number it sees.
( for i in $(seq 1 6); do head -c 1000 /dev/zero >> "$sb_dir/grow.bin"; sleep 0.25; done ) &
sb_grow=$!
SETTLE_MAX=40 sb_arm "grows then stops"       ok     "$sb_dir/grow.bin" 6000
wait "$sb_grow" 2>/dev/null
# grows PAST the deadline: the barrier must refuse rather than loop or grade mid-flight.
( for i in $(seq 1 40); do head -c 1000 /dev/zero >> "$sb_dir/forever.bin"; sleep 0.2; done ) &
sb_forever=$!
SETTLE_MAX=8  sb_arm "still growing at deadline" refuse "$sb_dir/forever.bin"
wait "$sb_forever" 2>/dev/null
rm -rf "$sb_dir"
if [ "$sb_fail" -ne 0 ]; then
    echo "BARRIER-SELFTEST-FAILED — the read barrier is broken, no measurement taken"; exit 1
fi
echo "[barrier-selftest] 5/5 arms reached (settle x2, refuse x3)"

# ------------------------------------------- grader selftest (Law 9: reach BOTH arms offline)
st_dir="$(mktemp -d)"
python3 - "$st_dir" <<'PY'
import sys, os, struct
d = sys.argv[1]
def wav(path, payload):
    hdr = (b'RIFF' + struct.pack('<I', 36+len(payload)) + b'WAVEfmt '
           + struct.pack('<IHHIIHH',16,1,2,44100,176400,4,16) + b'data'
           + struct.pack('<I', len(payload)))
    open(path,'wb').write(hdr+payload)
tone  = bytes((i*37) % 251 for i in range(4000))
tone2 = bytes((i*53) % 251 for i in range(4000))
wav(os.path.join(d,'tone_a.wav'),  tone)
wav(os.path.join(d,'tone_a2.wav'), tone)          # byte-identical copy
wav(os.path.join(d,'tone_b.wav'),  tone2)         # same length, different content
wav(os.path.join(d,'short.wav'),   tone[:2000])   # different length
wav(os.path.join(d,'silent.wav'),  b'\x00'*4000)  # one distinct byte
wav(os.path.join(d,'header.wav'),  b'')
PY
st_fail=0
selftest_arm() { # <fileA> <fileB> <expected-token> <label>
    got="$(cmp_pcm "$st_dir/$1" "$st_dir/$2" | awk '{print $1}')"
    if [ "$got" = "$3" ]; then echo "[selftest] $4 -> $got  OK"
    else echo "[selftest] $4 -> $got  EXPECTED $3  FAIL"; st_fail=1; fi
}
selftest_arm tone_a.wav  tone_a2.wav SAME "identical payloads"
selftest_arm tone_a.wav  tone_b.wav  DIFF "same length, different bytes"
selftest_arm tone_a.wav  short.wav   DIFF "different lengths"
selftest_arm tone_a.wav  silent.wav  BAD  "one side silent"
selftest_arm header.wav  tone_a.wav  BAD  "one side header-only"
selftest_arm absent.wav  tone_a.wav  BAD  "one side absent"
# the byte COUNT alone must NOT decide: equal counts reach both SAME and DIFF above.
rm -rf "$st_dir"
if [ "$st_fail" -ne 0 ]; then
    echo "SELFTEST-FAILED — grader is broken, no measurement taken"; exit 1
fi
echo "[selftest] 6/6 arms reached (SAME, DIFF x2, BAD x3)"

# ------------------------ pre-flight selftest (Law 9: BOTH directions, offline, no boots)
# Arm "held" is the REAL 3.269 shape, not a stand-in: a second process holding an open
# handle makes `rm` fail with "Device or resource busy" on this platform — the exact
# string run 1's log printed. It signals readiness with a .ready file and releases on a
# .release file (20 s self-limit), so nothing has to be killed: `kill` does not reach
# native Windows children anyway (3.252).
pf_dir="$(mktemp -d)"
pf_fail=0
pf_arm() { # <label> <ok|refuse> <path>
    if capture_writable "$3" >/dev/null 2>&1; then got=ok; else got=refuse; fi
    if [ "$got" = "$2" ]; then echo "[preflight-selftest] $1 -> $got  OK"
    else echo "[preflight-selftest] $1 -> $got  EXPECTED $2  FAIL"; pf_fail=1; fi
}
pf_arm "absent path"            ok     "$pf_dir/absent.wav"
printf 'stale' > "$pf_dir/stale.wav"
pf_arm "stale removable file"   ok     "$pf_dir/stale.wav"
printf 'x' > "$pf_dir/held.wav"
python3 - "$pf_dir/held.wav" <<'PY' &
import sys, os, time
p = sys.argv[1]
f = open(p, 'r+b')
open(p + '.ready', 'w').close()
t = 0.0
while t < 20.0 and not os.path.exists(p + '.release'):
    time.sleep(0.05); t += 0.05
f.close()
PY
pf_holder=$!
pf_w=0
while [ ! -e "$pf_dir/held.wav.ready" ] && [ "$pf_w" -lt 200 ]; do sleep 0.05; pf_w=$((pf_w+1)); done
if [ ! -e "$pf_dir/held.wav.ready" ]; then
    echo "[preflight-selftest] holder never opened the file — arm unreached  FAIL"; pf_fail=1
else
    pf_arm "file held by another process" refuse "$pf_dir/held.wav"
fi
: > "$pf_dir/held.wav.release"; wait "$pf_holder" 2>/dev/null
mkdir -p "$pf_dir/dir.wav"; printf 'y' > "$pf_dir/dir.wav/inside"
pf_arm "path is a non-empty dir"  refuse "$pf_dir/dir.wav"
rm -rf "$pf_dir"
if [ "$pf_fail" -ne 0 ]; then
    echo "PREFLIGHT-SELFTEST-FAILED — the pre-flight is broken, no measurement taken"; exit 1
fi
echo "[preflight-selftest] 4/4 arms reached (writable x2, refuse x2)"

# ------------------------------------------------------------------ the measurement
mkdir -p "$LOGDIR"

# PRE-FLIGHT, before the first boot: every file this run will write must be clearable and
# creatable RIGHT NOW. The .png/.serial live in LOGDIR and are written by the same guest,
# so they are checked on the same footing as the .wav.
if ! preflight_captures "$WAV_A" "$WAV_C" "$WAV_B" \
                        "$LOGDIR/a.png" "$LOGDIR/c.png" "$LOGDIR/b.png" \
                        "$LOGDIR/a.serial" "$LOGDIR/c.serial" "$LOGDIR/b.serial"; then
    echo "RIG-BUSY: a capture path could not be made writable BEFORE any boot — a QEMU"
    echo "          from a previous invocation is very likely still holding it (3.269"
    echo "          run 1). REFUSING: no boots taken, no verdict, nothing back-fitted."
    echo "          This is a refusal, NOT a retry — re-run once the prior guest is reaped."
    exit 4
fi

boot() { # <wav> <module> <tag>
    echo "[probe] boot $3: $2 -> $1"
    WAV="$1" ./scripts/audio-shot.sh "$LOGDIR/$3.png" "nia $2" >"$LOGDIR/$3.serial" 2>&1
    echo "[probe]   rig rc=$? wav=$(wc -c <"$1" 2>/dev/null || echo 0) bytes"
}
boot "$WAV_A" notify.wasm  a
boot "$WAV_C" notify.wasm  c
boot "$WAV_B" notify2.wasm b

# POST-FLIGHT READ BARRIER, before ANY grading. THE BOOT ORDER IS UNCHANGED (a,c,b): 3.272
# already ruled on the reorder and got NOTHING (P(0 in 9)=0.10), so shipping the reorder in
# the same tick as the barrier would confound two changes and make a 3/3 re-gate unreadable.
# One variable at a time.
#
# Every capture is barred, not just the last one: a,c were written earlier, so they settle at
# the minimum sample count and are the barrier's OWN in-run control — if THEY need extra
# samples too, the "only the last boot is exposed" model is wrong and the run says so. Order
# is boot order, so the last boot (b, the M case) is barred last.
sb_bad=0
for t in a c b; do
    case "$t" in a) w="$WAV_A" ;; b) w="$WAV_B" ;; c) w="$WAV_C" ;; esac
    if out="$(settle_capture "$w")"; then
        echo "[barrier] $t $w -> STATIONARY bytes=${out%% *} samples=${out##* }"
    else
        echo "[barrier] $t $w -> STILL CHANGING at deadline bytes=${out%% *} samples=${out##* }"
        sb_bad=1
    fi
done
if [ "$sb_bad" -ne 0 ]; then
    echo "RIG-UNSETTLED: a capture was still changing size when the barrier's deadline"
    echo "               expired. REFUSING: nothing graded, no verdict, no re-boot — a"
    echo "               mid-flush read is exactly the 4096-boundary truncation 3.267/3.271"
    echo "               measured, and grading it would back-fit the answer."
    exit 5
fi

# Rig sanity ONLY — never the verdict. The serial line proves the RIGHT MODULE ran,
# it says nothing about the audio (3.254: that line fooled docs/14).
for t in a c b; do
    if ! grep -q "asili voice" "$LOGDIR/$t.serial"; then
        echo "RIG-BROKEN: no '[asili voice]' line in $LOGDIR/$t.serial (speak() never ran)"
        tail -5 "$LOGDIR/$t.serial"; exit 1
    fi
done
if grep -q "KETTLE" "$LOGDIR/a.serial" || ! grep -q "KETTLE" "$LOGDIR/b.serial"; then
    echo "RIG-BROKEN: the two boots did not run different texts (module mix-up)"; exit 1
fi
echo "[probe] rig sane: three boots ran, and boot b spoke the SECOND text"

control="$(cmp_pcm "$WAV_A" "$WAV_C")"
echo "[probe] CONTROL notify.wasm vs notify.wasm: $control"
case "${control%% *}" in
    BAD)  echo "RIG-BROKEN: control boots ungradeable ($control)"; exit 1 ;;
    DIFF) echo "RIG-NOISY: two boots of the SAME text already differ ($control) — the"
          echo "           noise floor swamps the signal; NO verdict taken."; exit 1 ;;
esac

verdict="$(cmp_pcm "$WAV_A" "$WAV_B")"
echo "[probe] TEST notify.wasm vs notify2.wasm: $verdict"
case "${verdict%% *}" in
    BAD)  echo "RIG-BROKEN: test boots ungradeable ($verdict)"; exit 1 ;;
    DIFF) echo "SPEAK-TEXT-DEPENDENT: different text -> different PCM ($verdict)"; exit 0 ;;
    *)    echo "SPEAK-TEXT-INDEPENDENT: two different texts produced BYTE-IDENTICAL PCM ($verdict)"
          exit 10 ;;
esac
