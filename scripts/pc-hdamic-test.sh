#!/usr/bin/env bash
# PC track step 3, sound in (10 Nov 2026): the PC's own MICROPHONE through Intel HD Audio (kernel/src/pc/hda.rs).
#
# hda.rs could only play. A laptop's built-in microphone (or a mic jack, or line in) is an input pin on the
# same codec, reached by an ADC through its connection list (often through a mixer or input selector); the
# ADC writes into the controller's first INPUT stream. hda.rs now picks the input pin (built-in mic, then a
# mic jack, then line in) with a path to an ADC, powers, selects and unmutes that path (80% mic bias when the
# pin offers it), and records through the first input stream from a 64 KiB ring read behind its DMA position
# (LPIB). `mic` names the path, `mic record [1-3]` records up to 3 s and prints the level heard, `mic play`
# plays the recording back through the output path.
#
# One power-on with qemu-system-x86_64 directly: an ICH9 HD Audio controller (8086:293e) with QEMU's
# hda-micro codec (speaker + microphone), on the "none" audiodev (its microphone gives silence at the real
# rate, so the input DMA runs at 44.1 kHz). Graded on the serial side: the boot lines (output path and the
# microphone path), lshda (the input pin, the ADC, the input path), mic, `mic record 1` (44100 samples at
# 44100 Hz, the level line, silence said as silence), `mic play` (the 44100 samples played back through the
# output path), nothing stuck, no panic. Not measured here: real sound (QEMU's backend is silent).
#   scripts/pc-hdamic-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-hdamic-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY=python3; command -v python3 > /dev/null || PY=python

grade() { # grade <serial log>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxE "$1" <<< "$S"; }
    local P='pin [0-9]+ \((speaker|headphones|line out)\) <- DAC [0-9]+'
    local I='pin [0-9]+ \((built-in )?microphone\)( -> (mixer|selector) [0-9]+)* -> ADC [0-9]+'
    has "\[hda\] Intel ICH9 HD Audio at [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(8086:293e\), codec 0 1af4:[0-9a-f]{4}, sound out through $P" ||
        _f "no boot line naming the ICH9 controller and the output path"
    has "\[hda\] microphone: $I, input stream 1 \('mic record' to try it\)" || _f "no boot line naming the microphone path"
    has '  node [0-9]+ input pin: microphone(, built in)? \(config 0x[0-9a-f]{8}\)' || _f "lshda did not list the microphone pin as used"
    has '  node [0-9]+ audio input \(ADC\)' || _f "lshda did not list the ADC"
    has "lshda: sound comes in through $I, input stream 1, 44\.1 kHz 16-bit stereo" || _f "lshda did not give the input path at 44.1 kHz"
    has "mic: $I on input stream 1, 44\.1 kHz 16-bit; 'mic record' \(2 s, or 'mic record 1'\.\.'3'\) then 'mic play'" || _f "mic did not name the path"
    has '\[mic\] recording 1 s: speak now' || _f "mic record 1 did not start"
    has "\[hda\] recorded 44100 samples \(~1000 ms\) at 44100 Hz from $I: peak [0-9]+ of 32767, average [0-9]+" ||
        _f "the recording was not 44100 samples at 44.1 kHz from the microphone"
    # QEMU's none backend is silent: the level must say so (a real PC would say "sound was heard")
    has '\[hda\] silence: nothing was heard \(is the microphone muted or switched off\?\)' || _f "the silent recording was not called silence"
    has "\[hda\] recorded .*: peak ([0-9]|[1-5][0-9]|6[0-3]) of 32767, average [0-9]+" || _f "the silent recording has a loud peak"
    has '\[mic\] done; .mic play. plays it back' || _f "mic record did not finish"
    has '\[mic\] playing back 44100 samples recorded at 44100 Hz' || _f "mic play did not start with the 44100 samples"
    printf '%s\n' "$S" | sed -n '/^\[mic\] playing back/,$p' | grep -aqE "^\[hda\] played 44100 samples \(44 periods, ~1000 ms\) at 44100 Hz through $P\$" ||
        _f "the recording was not played back in full through the output path"
    has '\[mic\] played back' || _f "mic play did not finish"
    if printf '%s\n' "$S" | grep -aqE '^\[hda\] (the (input|output) stream|no microphone input|HD Audio controller found but not usable)|^mic: no '; then
        _f "a stream stuck, or the microphone or controller was refused"
    fi
    if printf '%s\n' "$S" | grep -aqE 'fell behind'; then _f "the recorder fell behind the DMA"; fi
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[hda] Intel ICH9 HD Audio at 00:02.0 (8086:293e), codec 0 1af4:0012, sound out through pin 3 (speaker) <- DAC 2
[hda] microphone: pin 5 (microphone) -> ADC 4, input stream 1 ('mic record' to try it)
[pc] MANSA-PC BOOT OK
afrisovera> lshda
lshda: Intel ICH9 HD Audio at 00:02.0 (8086:293e), HD Audio 1.0, 4 output / 4 input streams
lshda: 1 codec(s) answered (mask 0x0001)
lshda: codec 0: QEMU 1af4:0012, audio function group at node 1, 4 widgets
  node 2 audio output (DAC)
  node 3 pin: speaker (config 0x90100010)
  node 4 audio input (ADC)
  node 5 input pin: microphone (config 0x90a00020)
lshda: sound goes out through pin 3 (speaker) <- DAC 2, stream 1, 44.1 kHz 16-bit stereo
lshda: sound comes in through pin 5 (microphone) -> ADC 4, input stream 1, 44.1 kHz 16-bit stereo
afrisovera> mic
mic: pin 5 (microphone) -> ADC 4 on input stream 1, 44.1 kHz 16-bit; 'mic record' (2 s, or 'mic record 1'..'3') then 'mic play'
afrisovera> mic record 1
[mic] recording 1 s: speak now
[hda] recorded 44100 samples (~1000 ms) at 44100 Hz from pin 5 (microphone) -> ADC 4: peak 0 of 32767, average 0
[hda] silence: nothing was heard (is the microphone muted or switched off?)
[mic] done; 'mic play' plays it back
afrisovera> mic play
[mic] playing back 44100 samples recorded at 44100 Hz
[hda] played 44100 samples (44 periods, ~1000 ms) at 44100 Hz through pin 3 (speaker) <- DAC 2
[mic] played back"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-hdamic-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm model-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm built-in-via-mixer green "$(printf '%s\n' "$GREEN" | sed 's/pin 5 (microphone) -> ADC 4/pin 18 (built-in microphone) -> mixer 35 -> ADC 8/g; s/input pin: microphone (config/input pin: microphone, built in (config/')"
    arm no-mic-boot-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[hda\] microphone:')"
    arm mic-refused red "$(printf '%s\n' "$GREEN" | sed 's/^\[hda\] microphone:.*/&\n[hda] no microphone input: no microphone or line-in pin with a path to an ADC/')"
    arm line-in-path red "$(printf '%s\n' "$GREEN" | sed 's/pin 5 (microphone)/pin 5 (line in)/g')"
    arm pin-unused red "$(printf '%s\n' "$GREEN" | sed 's/(config 0x90a00020)/(config 0x90a00020), not used/')"
    arm no-adc red "$(printf '%s\n' "$GREEN" | grep -v 'audio input (ADC)')"
    arm in-48k red "$(printf '%s\n' "$GREEN" | sed 's/input stream 1, 44\.1 kHz/input stream 1, 48 kHz/')"
    arm no-record-start red "$(printf '%s\n' "$GREEN" | grep -v '^\[mic\] recording')"
    arm short-record red "$(printf '%s\n' "$GREEN" | sed 's/recorded 44100 samples (~1000 ms)/recorded 22050 samples (~500 ms)/')"
    arm loud-silence red "$(printf '%s\n' "$GREEN" | sed 's/peak 0 of 32767/peak 9000 of 32767/')"
    arm said-heard red "$(printf '%s\n' "$GREEN" | sed 's/^\[hda\] silence:.*/[hda] sound was heard/')"
    arm input-stuck red "$(printf '%s\n' "$GREEN" | sed 's/^\[mic\] recording 1 s: speak now$/&\n[hda] the input stream stopped moving at byte 0 (LPIB 0x0)/')"
    arm fell-behind red "$(printf '%s\n' "$GREEN" | sed 's/average 0$/average 0 (some sound lost: the recorder fell behind)/')"
    arm no-play red "$(printf '%s\n' "$GREEN" | grep -v '^\[hda\] played')"
    arm play-short red "$(printf '%s\n' "$GREEN" | sed 's/played 44100 samples (44 periods/played 22050 samples (22 periods/')"
    arm play-not-done red "$(printf '%s\n' "$GREEN" | grep -v '^\[mic\] played back')"
    arm panic red "$(printf '%s\nKERNEL PANIC\n' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 20 ]
}

echo "== PC sound in: the microphone through Intel HD Audio (hda.rs) =="
if ! selftest; then echo "PC HDAMIC TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC HDAMIC SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC HDAMIC TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC HDAMIC TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
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
[ -n "$OVMF" ] || { echo "PC HDAMIC TEST FAILED (harness: no x86-64 UEFI firmware)"; exit 1; }

W="$(mktemp -d "$T/pc-hdamic.XXXXXX")"
mkdir -p "$W/stick/EFI/BOOT"
cp "$PC_EFI" "$W/stick/EFI/BOOT/BOOTX64.EFI"

echo "  --- boot: ICH9 HD Audio + QEMU's hda-micro codec on the silent none audiodev; lshda, mic, mic record 1, mic play ---"
"$REALQ" -M q35 -m 256M -display none -nic none \
    -drive "if=pflash,format=raw,unit=0,readonly=on,file=$(winpath "$OVMF")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/stick")" \
    -audiodev "none,id=snd0" \
    -device ich9-intel-hda -device hda-micro,audiodev=snd0 \
    -serial "tcp:127.0.0.1:5486,server,nowait" -qmp "tcp:127.0.0.1:5487,server,nowait" &
Q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" "$PY" scripts/shell-demo.py 5486 5487 "$W/shot.png" \
    lshda mic "mic record 1" "mic play" > "$W/boot.log" 2>&1
kill "$Q" 2> /dev/null
wait "$Q" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[hda\]|^lshda|^  node|^\[mic\]|^mic|^afrisovera> ' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC HDAMIC TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC HDAMIC TEST FAILED"
exit 1
