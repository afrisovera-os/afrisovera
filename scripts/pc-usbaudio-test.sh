#!/usr/bin/env bash
# PC track step 3, sound (8 Nov 2026): USB audio -- headsets, USB speakers, a monitor's or dock's sound
# (kernel/src/pc/xhci.rs, Audio Class 1.0).
#
# A USB headset is the cheapest sound a PC can have and the only one on many thin clients and desk setups;
# until now the xHCI driver called it "audio" and left it alone. Now its playback interface is read (the
# best 16-bit PCM setting: stereo before mono, 44.1 kHz before 48 kHz), the device configured, switched to
# that setting, unmuted, and its isochronous OUT endpoint fed one packet per service interval (1 ms at full
# speed), resampled from the shared 44.1 kHz sound calls. pc/disk.rs sends `audio tone`, `say` and Nia's
# `speak()` there when a USB audio device is plugged in (before HD Audio, as other systems switch to it).
#
# One power-on with qemu-system-x86_64 directly: a qemu-xhci with QEMU's usb-audio (full speed, 48 kHz stereo
# only, 192-byte packets), recorded by the wav audiodev. Graded on the serial side: the boot line (48000 Hz
# stereo, endpoint 1 every 1000 us, unmuted), lsusb's port mark and audio line (every packet played, none
# missed), audio, audio tone (16384 samples as 1783x frames at 48 kHz in 37x packets) and say played through
# USB audio, usb-audio in devices, nothing stuck, no panic. Graded on the SOUND (the WAV file the device
# wrote, not a serial line): the tone is there as a square wave of at least 15000 frames at 441 Hz (+-10),
# and `say` added at least 4000 more loud frames with at least 50 different sample values.
#   scripts/pc-usbaudio-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbaudio-test.sh --selftest   # grader selftest only
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
# the device's 48 kHz reaches the file through QEMU's resampler, which puts one or two in-between
# samples on each edge of the square wave: a dip of at most 2 frames between loud ones still counts
tone = [abs(s) >= 2000 for s in left]
for k in range(1, n - 1):
    if not tone[k] and tone[k - 1] and (tone[k + 1] or (k + 2 < n and tone[k + 2])):
        tone[k] = True
best, bs, cur, cs = 0, 0, 0, 0
for k, s in enumerate(left):
    if tone[k]:
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
    has() { printf '%s\n' "$S" | grep -aqxE "$1"; }
    has '\[xhci\] port [0-9]+: USB audio 46f4:0002 \(full speed\), slot [0-9]+, 2 channel\(s\) 16-bit at 48000 Hz, isochronous OUT endpoint 1 every 1000 us, unmuted' ||
        _f "no boot line for the USB audio device (48 kHz stereo, endpoint 1 every 1 ms, unmuted)"
    has '  port [0-9]+: connected, enabled, speed [0-9]+  <- audio' || _f "lsusb did not mark the audio device's port"
    has '  audio: slot [0-9]+, 48000 Hz 2-channel 16-bit, 2 stream\(s\), [0-9]+ packets sent, [0-9]+ played, 0 missed' ||
        _f "lsusb's audio line is missing, not after two streams, or packets were missed"
    printf '%s\n' "$S" | grep -aE '^  audio: slot' | sed -nE 's/.* ([0-9]+) packets sent, ([0-9]+) played.*/\1 \2/p' | { read -r a b; [ -n "${a:-}" ] && [ "$a" = "$b" ] && [ "$a" -ge 900 ]; } ||
        _f "lsusb: not every packet sent was played, or too few (want >= 900 for the tone and say)"
    has "\[audio\] USB audio online: slot [0-9]+, 48000 Hz 2-channel 16-bit, 192 bytes per packet at most; 'audio tone' to play" || _f "audio did not report USB audio"
    has '\[audio\] streaming a test tone through the USB audio device\.\.\.' || _f "audio tone did not go to USB audio"
    has '\[usb-audio\] played 16384 samples \(~371 ms\) as 1783[0-9] frames at 48000 Hz in 37[0-9] packets' || _f "the tone was not played in full (16384 samples -> ~17834 frames, ~372 packets)"
    has '\[audio\] done.*' || _f "audio tone did not finish"
    has '\[say\] speaking: "hello"' || _f "say did not start"
    printf '%s\n' "$S" | sed -n '/^\[say\] speaking: "hello"$/,$p' | grep -aqE '^\[usb-audio\] played ([5-9][0-9]{3}|[1-9][0-9]{4,}) samples \(~[0-9]+ ms\) as [0-9]+ frames at 48000 Hz in [0-9]+ packets$' ||
        _f "say was not played through USB audio"
    has 'usb-audio +[a-z]+ +0x0+ +OK' || _f "usb-audio is not in devices"
    if printf '%s\n' "$S" | grep -aqE '^\[xhci\] audio: the device stopped|^\[xhci\] port [0-9]+: USB audio .*(not driven|do not fit)'; then _f "the USB audio stream stuck or the device was refused"; fi
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[xhci] port 5: USB audio 46f4:0002 (full speed), slot 1, 2 channel(s) 16-bit at 48000 Hz, isochronous OUT endpoint 1 every 1000 us, unmuted
[pc] MANSA-PC BOOT OK
afrisovera> audio
[audio] USB audio online: slot 1, 48000 Hz 2-channel 16-bit, 192 bytes per packet at most; 'audio tone' to play
afrisovera> audio tone
[audio] streaming a test tone through the USB audio device...
[usb-audio] played 16384 samples (~371 ms) as 17833 frames at 48000 Hz in 372 packets
[audio] done — played by the USB headset or speaker
afrisovera> say hello
[say] speaking: \"hello\"
[usb-audio] played 25578 samples (~580 ms) as 27840 frames at 48000 Hz in 580 packets
[asili voice] synthesized 25578 samples
afrisovera> lsusb
usb: xHCI controller 00:02.0, 8 ports, 1 connected
  port 5: connected, enabled, speed 1  <- audio
  audio: slot 1, 48000 Hz 2-channel 16-bit, 2 stream(s), 952 packets sent, 952 played, 0 missed
afrisovera> devices
NAME               CLASS      MMIO          STATUS
usb-audio          audio      0x0000000000  OK"
WGREEN='wav: frames=70000 rate=44100 ch=2 loud=42000 tone_frames=16384 tone_hz=441.0 voice_loud=25000 voice_distinct=900'
HDALINE="[audio] HD Audio online: QEMU codec 1af4:0012, out through pin 3 (line out) <- DAC 2; 'audio tone' to play"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbaudio-selftest.XXXXXX")" || return 1
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
    arm no-boot-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] port 5: USB audio')"
    arm muted red "$(printf '%s\n' "$GREEN" | sed 's/ every 1000 us, unmuted/ every 1000 us/')"
    arm mono red "$(printf '%s\n' "$GREEN" | sed 's/2 channel(s) 16-bit/1 channel(s) 16-bit/')"
    arm uac2-refused red "$(printf '[xhci] port 5: USB audio 46f4:0002 is Audio Class 2.0 -- not driven yet\n%s\n' "$GREEN" | grep -v 'isochronous OUT')"
    arm no-port-mark red "$(printf '%s\n' "$GREEN" | sed 's/  <- audio$//')"
    arm missed red "$(printf '%s\n' "$GREEN" | sed 's/952 played, 0 missed/950 played, 2 missed/')"
    arm not-all-played red "$(printf '%s\n' "$GREEN" | sed 's/952 played/940 played/')"
    arm few-packets red "$(printf '%s\n' "$GREEN" | sed 's/952 packets sent, 952 played/400 packets sent, 400 played/')"
    arm audio-hda red "$(printf '%s\n' "$GREEN" | grep -v '^\[audio\] USB audio online'; echo "$HDALINE")"
    arm tone-short red "$(printf '%s\n' "$GREEN" | sed 's/played 16384 samples (~371 ms) as 17833 frames at 48000 Hz in 372/played 8192 samples (~185 ms) as 8916 frames at 48000 Hz in 186/')"
    arm tone-44k red "$(printf '%s\n' "$GREEN" | sed 's/as 17833 frames at 48000 Hz/as 16384 frames at 44100 Hz/')"
    arm tone-not-done red "$(printf '%s\n' "$GREEN" | grep -v '^\[audio\] done')"
    arm say-silent red "$(printf '%s\n' "$GREEN" | grep -v '^\[usb-audio\] played 25578')"
    arm no-device-row red "$(printf '%s\n' "$GREEN" | grep -v '^usb-audio')"
    arm stuck red "$(printf '%s\n[xhci] audio: the device stopped taking packets (40 sent, 0 done)\n' "$GREEN")"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    arm wav-missing red "$GREEN" "wav: missing"
    arm wav-silent red "$GREEN" "wav: frames=70000 rate=44100 ch=2 loud=0 tone_frames=0 tone_hz=0.0 voice_loud=0 voice_distinct=0"
    arm wav-tone-cut red "$GREEN" "${WGREEN/tone_frames=16384/tone_frames=9000}"
    arm wav-tone-fast red "$GREEN" "${WGREEN/tone_hz=441.0/tone_hz=480.1}"
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
edge = lambda n, half: [(1895 if k % half == 0 and k else 8000) * (1 if (k // half) % 2 == 0 else -1) for k in range(n)]
w('edges.wav', [0] * 3000 + edge(16384, 50) + [0] * 2000 + voice + [0] * 1000)
w('gappy.wav', [0] * 3000 + sq(4000, 50) + [0] * 10 + sq(4000, 50) + [0] * 10 + sq(4000, 50) + [0] * 10 + sq(4384, 50) + [0] * 2000 + voice)
EOF
    arm measured-good green "$GREEN" "$(wav "$D/good.wav")"
    arm measured-silent red "$GREEN" "$(wav "$D/silent.wav")"
    arm measured-600hz red "$GREEN" "$(wav "$D/high.wav")"
    arm measured-resampled-edges green "$GREEN" "$(wav "$D/edges.wav")"
    arm measured-gaps red "$GREEN" "$(wav "$D/gappy.wav")"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 30 ]
}

echo "== PC sound: USB audio (xhci.rs, Audio Class 1.0) =="
if ! selftest; then echo "PC USBAUDIO TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBAUDIO SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBAUDIO TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBAUDIO TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] || { echo "PC USBAUDIO TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }

W="$(mktemp -d "$T/pc-usbaudio.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: qemu-xhci + QEMU's usb-audio, recorded to a WAV file; audio, audio tone, say hello, lsusb, devices ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -audiodev "wav,id=snd0,path=$(winpath "$W/out.wav")" \
    -device qemu-xhci -device usb-audio,audiodev=snd0 \
    -serial "tcp:127.0.0.1:5620,server,nowait" -qmp "tcp:127.0.0.1:5621,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5620 5621 "$W/shot.png" \
    audio "audio tone" "say hello" lsusb devices > "$W/boot.log" 2>&1
# a clean QEMU exit, so the wav audiodev finishes its file (header sizes, last buffer)
"$PY" - << 'EOF' 2> /dev/null
import socket
try:
    s = socket.create_connection(('127.0.0.1', 5621), timeout=5)
    s.recv(4096)
    s.sendall(b'{"execute":"qmp_capabilities"}\n'); s.recv(4096)
    s.sendall(b'{"execute":"quit"}\n'); s.recv(4096)
except Exception:
    pass
EOF
for _ in $(seq 1 50); do kill -0 "$Q" 2> /dev/null || break; sleep 0.2; done
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[xhci\]|^\[usb-audio\]|^usb:|^  port|^  audio|^\[audio\]|^\[say\]|^usb-audio|^afrisovera> ' | sed 's/^/  | /'
WS="$(wav "$W/out.wav")"
echo "  | $WS"

echo "  --- grading ---"
ok=1
grade "$W/boot.log" || ok=0
grade_wav "$WS" || ok=0
if [ "$ok" = 1 ]; then
    rm -rf "$W"
    echo "PC USBAUDIO TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBAUDIO TEST FAILED"
exit 1
