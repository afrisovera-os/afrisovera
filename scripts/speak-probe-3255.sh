#!/usr/bin/env bash
# 3.255 falsify-first probe, PRE-REGISTERED at 3.254 (docs/finding-3.254-speak-makes-no-sound.md).
#
# Claim under test: `speak()` (HOST_SPEAK, nia.rs) makes NO SOUND — it only kprintln's
# "(would speak aloud)". All 685 green suites grade that serial line, so the silence is
# invisible to them.
#
# THIS PROBE GRADES THE WAV, NOT THE SERIAL LINE. The serial line is exactly the evidence
# that fooled docs/14. It CALLS scripts/audio-shot.sh unedited (2.765 rule 5) with the
# already-proven QEMU line (-audiodev wav + -device virtio-sound-device) and drives the
# guest to run notify.wasm, whose only body is speak(0, len).
#
#   exit 10 SPEAK-SILENT  = expected on the UNTOUCHED tree (the finding reproduces)
#   exit  0 SPEAK-AUDIBLE = expected only AFTER HOST_SPEAK is wired to sound_play_tone()
#   exit  1 RIG-BROKEN    = speak() never ran / boot or shell rig failed — NOT a verdict
#
# A tone is NOT speech: flipping this to 0 buys an audible speak(), not TTS (docs/14).
set -uo pipefail

WAV="${WAV:-.speak-3255.wav}"
PNG="${PNG:-.speak-3255.png}"
LOG="${LOG:-.speak-3255.serial}"

# ---------------------------------------------------------------- the grader
# Silence = every byte of the PCM payload (past the 44-byte RIFF header) identical.
# Absent or header-only file = silent too. Prints SILENT|AUDIBLE|MISSING + detail.
grade_wav() {
python3 - "$1" <<'PY'
import sys, os
p = sys.argv[1]
if not os.path.exists(p):
    print("MISSING no-file"); sys.exit(0)
b = open(p, 'rb').read()
if len(b) <= 44:
    print("MISSING bytes=%d (header-only or empty)" % len(b)); sys.exit(0)
pcm = b[44:]
vals = set(pcm)
if len(vals) <= 1:
    print("SILENT pcm=%d distinct=%d" % (len(pcm), len(vals)))
else:
    print("AUDIBLE pcm=%d distinct=%d" % (len(pcm), len(vals)))
PY
}

# ------------------------------------------------- grader selftest (Law 9: both arms)
# Reach BOTH verdicts offline, before spending a boot, on hand-built WAVs.
st_dir="$(mktemp -d)"
python3 - "$st_dir" <<'PY'
import sys, os, struct
d = sys.argv[1]
def wav(path, payload):
    hdr = b'RIFF' + struct.pack('<I', 36+len(payload)) + b'WAVEfmt ' + struct.pack('<IHHIIHH',16,1,2,44100,176400,4,16) + b'data' + struct.pack('<I', len(payload))
    open(path,'wb').write(hdr+payload)
wav(os.path.join(d,'silent.wav'), b'\x00'*4000)
wav(os.path.join(d,'tone.wav'), bytes((i*37) % 251 for i in range(4000)))
wav(os.path.join(d,'headeronly.wav'), b'')
PY
st_fail=0
for arm in "silent.wav SILENT" "tone.wav AUDIBLE" "headeronly.wav MISSING"; do
    f="${arm%% *}"; want="${arm##* }"
    got="$(grade_wav "$st_dir/$f" | awk '{print $1}')"
    if [ "$got" = "$want" ]; then
        echo "[selftest] $f -> $got  OK"
    else
        echo "[selftest] $f -> $got  EXPECTED $want  FAIL"; st_fail=1
    fi
done
got="$(grade_wav "$st_dir/does-not-exist.wav" | awk '{print $1}')"
[ "$got" = "MISSING" ] && echo "[selftest] absent-file -> MISSING  OK" || { echo "[selftest] absent-file -> $got EXPECTED MISSING FAIL"; st_fail=1; }
rm -rf "$st_dir"
if [ "$st_fail" -ne 0 ]; then
    echo "SELFTEST-FAILED — grader is broken, no measurement taken"; exit 1
fi
echo "[selftest] 4/4 arms reached (SILENT, AUDIBLE, MISSING x2)"

# ------------------------------------------------------------------ the measurement
rm -f "$WAV" "$LOG"
echo "[probe] booting the audio rig and running notify.wasm (speak() only)..."
WAV="$WAV" ./scripts/audio-shot.sh "$PNG" "nia notify.wasm" >"$LOG" 2>&1
rig_rc=$?

# Did speak() actually run? (rig sanity ONLY — never the verdict.)
if ! grep -q "asili voice" "$LOG"; then
    echo "RIG-BROKEN: no '[asili voice]' line in $LOG (speak() never ran, rig rc=$rig_rc)"
    tail -5 "$LOG"
    exit 1
fi
echo "[probe] speak() ran (serial line present) — now grading the WAV, not that line"

verdict="$(grade_wav "$WAV")"
echo "[probe] wav $WAV: $verdict"
case "${verdict%% *}" in
    AUDIBLE) echo "SPEAK-AUDIBLE: speak() produced PCM (${verdict#* })"; exit 0 ;;
    *)       echo "SPEAK-SILENT: speak() printed but produced no sound (${verdict})"; exit 10 ;;
esac
