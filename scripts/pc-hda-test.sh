#!/usr/bin/env bash
# PC track step 3, sound (7 Nov 2026): Intel HD Audio: the PC's own sound chip (kernel/src/pc/hda.rs).
#
# Every PC and laptop has an HD Audio controller with a codec behind it; until now the PC kernel could only
# play through QEMU's virtio-sound, which no real PC has. hda.rs resets the controller, talks to the codec
# through the CORB/RIRB rings, picks an analog output pin (speaker, headphones, line out) and the DAC behind
# it, and plays 44.1 kHz 16-bit stereo from a cyclic buffer refilled behind the DMA position. pc/disk.rs
# sends the shared sound calls there when there is no virtio-sound device, so `audio tone`, `say` and Nia's
# `speak()` sound through it unchanged.
#
# One power-on with qemu-system-x86_64 directly: an ICH9 HD Audio controller (8086:293e) with QEMU's
# hda-output codec (one line-out pin, one DAC), recorded by the wav audiodev. Graded on the serial side: the
# boot line, lshda (controller, one codec, the pin and DAC, the path), audio, audio tone and say played
# through HD Audio with the right sample counts, intel-hda in devices, nothing stuck, no panic. Graded on the
# SOUND (the WAV file the codec wrote, not a serial line): the tone is there as a square wave of at least
# 15000 frames at 441 Hz (+-10), and `say` added at least 4000 more loud frames with at least 50 different
# sample values (speech, not a copy of the tone or silence).
# Since 12 Nov 2026 also `lshda late`: a codec answer left unread in the RIRB (as after a command that timed
# out) must be dropped, not taken as the next command's answer; graded on the line it prints.
#   scripts/pc-hda-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-hda-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python

# wav <file>: one line "wav: frames=N rate=R ch=C loud=L tone_frames=T tone_hz=H voice_loud=V voice_distinct=D"
wav() {
    "$PY" - "$1" << 'EOF'
import struct, sys
try:
    b = open(sys.argv[1], 'rb').read()
except OSError:
    print('wav: missing'); sys.exit(0)
if len(b) < 12 or b[:4] != b'RIFF' or b[8:12] != b'WAVE':
    print('wav: not a wav file'); sys.exit(0)
i, ch, rate, bits, data = 12, 0, 0, 0, None
while i + 8 <= len(b):
    cid, n = b[i:i+4], struct.unpack('<I', b[i+4:i+8])[0]
    if cid == b'fmt ':
        ch, rate, bits = struct.unpack('<H', b[i+10:i+12])[0], struct.unpack('<I', b[i+12:i+16])[0], struct.unpack('<H', b[i+22:i+24])[0]
    if cid == b'data':
        data = b[i+8:]  # to the end: the size field may not be final if QEMU was killed
        break
    i += 8 + n + (n & 1)
if data is None or ch == 0 or bits != 16:
    print('wav: no 16-bit data'); sys.exit(0)
fs = 2 * ch
n = len(data) // fs
left = [struct.unpack_from('<h', data, k * fs)[0] for k in range(n)]
loud = sum(1 for s in left if abs(s) >= 1000)
best, bs, cur, cs = 0, 0, 0, 0
for k, s in enumerate(left):
    if abs(s) >= 2000:
        if cur == 0:
            cs = k
        cur += 1
        if cur > best:
            best, bs = cur, cs
    else:
        cur = 0
changes = sum(1 for k in range(bs + 1, bs + best) if (left[k] > 0) != (left[k - 1] > 0))
hz = changes / 2 * rate / best if best else 0
rest = [s for k, s in enumerate(left) if not (bs <= k < bs + best) and abs(s) >= 1000]
print('wav: frames=%d rate=%d ch=%d loud=%d tone_frames=%d tone_hz=%.1f voice_loud=%d voice_distinct=%d'
      % (n, rate, ch, loud, best, hz, len(rest), len(set(rest))))
EOF
}

grade_wav() { # grade_wav <stats line>
    local fail=0 L="$1" tf hz vl vd
    _w() { echo "  FAIL: $*"; fail=1; }
    printf '%s\n' "$L" | grep -qE '^wav: frames=' || { echo "  FAIL: no sound file to grade ($L)"; return 1; }
    tf="$(printf '%s\n' "$L" | sed -nE 's/.* tone_frames=([0-9]+) .*/\1/p')"
    hz="$(printf '%s\n' "$L" | sed -nE 's/.* tone_hz=([0-9]+)\.[0-9] .*/\1/p')"
    vl="$(printf '%s\n' "$L" | sed -nE 's/.* voice_loud=([0-9]+) .*/\1/p')"
    vd="$(printf '%s\n' "$L" | sed -nE 's/.* voice_distinct=([0-9]+)$/\1/p')"
    [ "${tf:-0}" -ge 15000 ] || _w "the tone is not in the sound file (longest loud run ${tf:-?} frames, want >= 15000)"
    { [ "${hz:-0}" -ge 431 ] && [ "${hz:-0}" -le 451 ]; } || _w "the tone is not 441 Hz (${hz:-?} Hz)"
    [ "${vl:-0}" -ge 4000 ] || _w "say left too little sound (${vl:-?} loud frames besides the tone, want >= 4000)"
    [ "${vd:-0}" -ge 50 ] || _w "say's sound is not speech-like (${vd:-?} distinct values, want >= 50)"
    return "$fail"
}

grade() { # grade <serial log>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxE "$1" <<< "$S"; }
    local P='pin [0-9]+ \(line out\) <- DAC [0-9]+'
    has "\[hda\] Intel ICH9 HD Audio at [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(8086:293e\), codec 0 1af4:[0-9a-f]{4}, sound out through $P" ||
        _f "no boot line naming the ICH9 controller, the QEMU codec and the line-out path"
    has 'lshda: Intel ICH9 HD Audio at [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(8086:293e\), HD Audio 1\.0, [1-9][0-9]* output / [0-9]+ input streams' ||
        _f "lshda did not name the controller"
    has 'lshda: 1 codec\(s\) answered \(mask 0x0001\)' || _f "lshda did not count one codec"
    has 'lshda: codec 0: QEMU 1af4:[0-9a-f]{4}, audio function group at node [0-9]+, [1-9][0-9]* widgets' || _f "lshda did not describe the codec"
    has '  node [0-9]+ pin: line out \(config 0x[0-9a-f]{8}\)' || _f "lshda did not list the line-out pin as used"
    has '  node [0-9]+ audio output \(DAC\)' || _f "lshda did not list the DAC"
    has "lshda: sound goes out through $P, stream 1, 44\.1 kHz 16-bit stereo" || _f "lshda did not give the output path at 44.1 kHz"
    has "\[audio\] HD Audio online: QEMU codec 1af4:[0-9a-f]{4}, out through $P; 'audio tone' to play" || _f "audio did not report HD Audio"
    has '\[audio\] streaming a test tone through Intel HD Audio\.\.\.' || _f "audio tone did not go to HD Audio"
    has "\[hda\] played 16384 samples \(16 periods, ~371 ms\) at 44100 Hz through $P" || _f "the tone was not played in full (16384 samples)"
    has '\[audio\] done.*' || _f "audio tone did not finish"
    has '\[say\] speaking: "hello"' || _f "say did not start"
    printf '%s\n' "$S" | sed -n '/^\[say\] speaking: "hello"$/,$p' | grep -aqE "^\[hda\] played ([5-9][0-9]{3}|[1-9][0-9]{4,}) samples \([0-9]+ periods, ~[0-9]+ ms\) at 44100 Hz through $P\$" ||
        _f "say was not played through HD Audio"
    has 'intel-hda +[a-z]+ +0x0+ +OK' || _f "intel-hda is not in devices"
    has 'lshda late: an unread answer was left in the ring; the next command got (0x[0-9a-f]{8}), then \1 \(codec id \1\), [1-9][0-9]* dropped: OK' ||
        _f "lshda late: a late codec answer was taken as the next command's answer"
    if printf '%s\n' "$S" | grep -aqE '^lshda: late codec answers dropped'; then _f "late codec answers came in normal use"; fi
    if printf '%s\n' "$S" | grep -aqE '^\[hda\] (the output stream|HD Audio controller found but not usable)'; then _f "the HD Audio stream stuck or the controller was refused"; fi
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[hda] Intel ICH9 HD Audio at 00:02.0 (8086:293e), codec 0 1af4:0012, sound out through pin 3 (line out) <- DAC 2
[pc] MANSA-PC BOOT OK
afrisovera> lshda
lshda: Intel ICH9 HD Audio at 00:02.0 (8086:293e), HD Audio 1.0, 4 output / 4 input streams
lshda: 1 codec(s) answered (mask 0x0001)
lshda: codec 0: QEMU 1af4:0012, audio function group at node 1, 2 widgets
  node 2 audio output (DAC)
  node 3 pin: line out (config 0x01014010)
lshda: sound goes out through pin 3 (line out) <- DAC 2, stream 1, 44.1 kHz 16-bit stereo
afrisovera> audio
[audio] HD Audio online: QEMU codec 1af4:0012, out through pin 3 (line out) <- DAC 2; 'audio tone' to play
afrisovera> audio tone
[audio] streaming a test tone through Intel HD Audio...
[hda] played 16384 samples (16 periods, ~371 ms) at 44100 Hz through pin 3 (line out) <- DAC 2
[audio] done: captured by the wav audiodev if attached
afrisovera> say hello
[say] speaking: \"hello\"
[hda] played 25578 samples (25 periods, ~580 ms) at 44100 Hz through pin 3 (line out) <- DAC 2
[asili voice] synthesized 25578 samples
afrisovera> devices
NAME               CLASS      MMIO          STATUS
intel-hda          audio      0x0000000000  OK
afrisovera> lshda late
lshda late: an unread answer was left in the ring; the next command got 0x1af40012, then 0x1af40012 (codec id 0x1af40012), 1 dropped: OK"
WGREEN='wav: frames=70000 rate=44100 ch=2 loud=42000 tone_frames=16384 tone_hz=441.0 voice_loud=25000 voice_distinct=900'

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-hda-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <serial text> [wav stats]
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1 && grade_wav "${4-$WGREEN}" >> "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-boot-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[hda\] Intel ICH9')"
    arm hdmi-path red "$(printf '%s\n' "$GREEN" | sed 's/pin 3 (line out)/pin 3 (digital out)/g')"
    arm no-lshda red "$(printf '%s\n' "$GREEN" | grep -v '^lshda: Intel')"
    arm two-codecs red "$(printf '%s\n' "$GREEN" | sed 's/1 codec(s) answered (mask 0x0001)/2 codec(s) answered (mask 0x0003)/')"
    arm pin-unused red "$(printf '%s\n' "$GREEN" | sed 's/(config 0x01014010)/(config 0x01014010), not used/')"
    arm no-dac red "$(printf '%s\n' "$GREEN" | grep -v 'audio output (DAC)')"
    arm 48k red "$(printf '%s\n' "$GREEN" | sed 's/44\.1 kHz 16-bit/48 kHz 16-bit/')"
    arm audio-virtio red "$(printf '%s\n' "$GREEN" | sed 's/^\[audio\] HD Audio online.*/[audio] no virtio-sound device (boot QEMU with -device virtio-sound-device,audiodev=snd0)/')"
    arm tone-short red "$(printf '%s\n' "$GREEN" | sed 's/played 16384 samples (16 periods/played 8192 samples (8 periods/')"
    arm tone-not-done red "$(printf '%s\n' "$GREEN" | grep -v '^\[audio\] done')"
    arm say-silent red "$(printf '%s\n' "$GREEN" | grep -v '^\[hda\] played 25578')"
    arm no-device-row red "$(printf '%s\n' "$GREEN" | grep -v '^intel-hda')"
    arm stuck red "$(printf '%s\n' "$GREEN" | sed 's/^\[audio\] done.*/&\n[hda] the output stream stopped moving at byte 4096 (LPIB 0x1000)/')"
    arm late-shifted red "$(printf '%s\n' "$GREEN" | sed 's/got 0x1af40012, then 0x1af40012/got 0x00010001, then 0x1af40012/; s/dropped: OK/dropped: WRONG/')"
    arm late-shifted-ok red "$(printf '%s\n' "$GREEN" | sed 's/got 0x1af40012, then 0x1af40012/got 0x00010001, then 0x1af40012/')"
    arm late-none-dropped red "$(printf '%s\n' "$GREEN" | sed 's/, 1 dropped: OK/, 0 dropped: OK/')"
    arm late-in-use red "$(printf '%s\n' "$GREEN" | sed 's/^lshda: sound goes out.*/&\nlshda: late codec answers dropped before a command: 3/')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    arm wav-missing red "$GREEN" "wav: missing"
    arm wav-silent red "$GREEN" "wav: frames=70000 rate=44100 ch=2 loud=0 tone_frames=0 tone_hz=0.0 voice_loud=0 voice_distinct=0"
    arm wav-tone-cut red "$GREEN" "${WGREEN/tone_frames=16384/tone_frames=9000}"
    arm wav-tone-48k red "$GREEN" "${WGREEN/tone_hz=441.0/tone_hz=480.1}"
    arm wav-no-voice red "$GREEN" "${WGREEN/voice_loud=25000/voice_loud=300}"
    arm wav-voice-flat red "$GREEN" "${WGREEN/voice_distinct=900/voice_distinct=2}"
    # the measuring side, on WAV files made here: a tone + speech-like sound, silence, a 600 Hz tone
    "$PY" - "$D" << 'EOF'
import math, struct, sys, os
def w(name, frames):
    data = b''.join(struct.pack('<hh', s, s) for s in frames)
    hdr = b'RIFF' + struct.pack('<I', 36 + len(data)) + b'WAVEfmt ' + struct.pack('<IHHIIHH', 16, 1, 2, 44100, 44100 * 4, 4, 16) + b'data' + struct.pack('<I', len(data))
    open(os.path.join(sys.argv[1], name), 'wb').write(hdr + data)
sq = lambda n, half: [8000 if (k // half) % 2 == 0 else -8000 for k in range(n)]
voice = [int(6000 * math.sin(k * 0.07) * math.sin(k * 0.003)) for k in range(20000)]
w('good.wav', [0] * 3000 + sq(16384, 50) + [0] * 2000 + voice + [0] * 1000)
w('silent.wav', [0] * 40000)
w('high.wav', [0] * 3000 + sq(16384, 37) + [0] * 2000 + voice)
EOF
    arm measured-good green "$GREEN" "$(wav "$D/good.wav")"
    arm measured-silent red "$GREEN" "$(wav "$D/silent.wav")"
    arm measured-600hz red "$GREEN" "$(wav "$D/high.wav")"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 30 ]
}

echo "== PC sound: Intel HD Audio (hda.rs) =="
if ! selftest; then echo "PC HDA TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC HDA SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC HDA TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC HDA TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"

REALQ="$(command -v qemu-system-x86_64)"
QDIR="$(dirname "$REALQ")"
OVMF=""
for d in "$QDIR/../share/qemu" "$QDIR/share" /usr/share/qemu; do
    [ -z "$OVMF" ] && [ -r "$d/edk2-x86_64-code.fd" ] && OVMF="$d/edk2-x86_64-code.fd"
done
for f in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    [ -z "$OVMF" ] && [ -r "$f" ] && OVMF="$f"
done
[ -n "$OVMF" ] || { echo "PC HDA TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }

W="$(mktemp -d "$T/pc-hda.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: ICH9 HD Audio + QEMU's hda-output codec, recorded to a WAV file; lshda, audio, audio tone, say hello, devices, lshda late ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -audiodev "wav,id=snd0,path=$(winpath "$W/out.wav")" \
    -device ich9-intel-hda -device hda-output,audiodev=snd0 \
    -serial "tcp:127.0.0.1:5482,server,nowait" -qmp "tcp:127.0.0.1:5483,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5482 5483 "$W/shot.png" \
    lshda audio "audio tone" "say hello" devices "lshda late" > "$W/boot.log" 2>&1
# a clean QEMU exit, so the wav audiodev finishes its file (header sizes, last buffer)
"$PY" - << 'EOF' 2> /dev/null
import socket
try:
    s = socket.create_connection(('127.0.0.1', 5483), timeout=5)
    s.recv(4096)
    s.sendall(b'{"execute":"qmp_capabilities"}\n'); s.recv(4096)
    s.sendall(b'{"execute":"quit"}\n'); s.recv(4096)
except Exception:
    pass
EOF
for _ in $(seq 1 50); do kill -0 "$Q" 2> /dev/null || break; sleep 0.2; done
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[hda\]|^lshda|^  node|^\[audio\]|^\[say\]|^intel-hda|^afrisovera> ' | sed 's/^/  | /'
WS="$(wav "$W/out.wav")"
echo "  | $WS"

echo "  --- grading ---"
ok=1
grade "$W/boot.log" || ok=0
grade_wav "$WS" || ok=0
if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC HDA TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC HDA TEST FAILED"
exit 1
