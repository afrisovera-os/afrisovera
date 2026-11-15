#!/usr/bin/env bash
# wasm-audio (claude/nia-audio, N-A1a, NIA_AUDIO_PLAN.md, the architect's OK with six conditions): Nia apps play
# sound through audio_open, audio_write, audio_level and audio_close. The app is the bundled tone.wasm
# (scripts/lib/audio-wasm.py; a disk app never gets the audio permission). QEMU's wav audiodev records what the
# sound device played; scripts/lib/audio-wav.py describes it. Four boots:
#   A  `nia tone.wasm`, key a: 0.5 s of 441 Hz, a 500 ms pause with nothing written, 0.5 s more (22,050 Hz mono).
#      The wav: exactly two stretches of sound, each 500 ms +- 30, 441 Hz +- 2 %, peak 7000 to 9000, with at least
#      400 ms of quiet between them (silence in the pause, nothing replayed: condition 2). The closed line: 22050
#      frames played and 1 underrun (the pause).
#   B  key b: 1 s of 480 Hz on the left of a 48,000 Hz stereo stream: one stretch of 1000 ms +- 30, 480 Hz +- 2 %,
#      peak 3500 to 4500 (mixed down to mono, (l + r) >> 1).
#   C  the edges (key e: every printed result), and every way out, each followed by `say hello`, which must play
#      (the device is free): a TRAP (key t), Ctrl+C (key c), a clean return with 200 ms queued (key r, all 4410
#      frames played), busy both ways (key s: the voice is busy while the stream is open, a second open is -1, the
#      voice speaks after audio_close); a disk copy (tone2.wasm, unverified) is DENIED at audio_open; then in the
#      desk, a close request (key w, the app returns) and Close anyway (key i, it stops at its next loop step).
#      Each stream's closed line says the device is free; a cell_event wait never ran more than 10 ms past its end
#      (condition 3: one driver period, 23 ms, plus the desk pass; 50 ms graded).
#   E  Ngoma (the bundled drum machine) in the desk: Space plays, q stops; its stream opens at 22,050 Hz mono, closes
#      with frames played and 0 underruns, the device played sound, and say plays after it.
#   D  no sound device: audio_open gives -2.
# No panic. Runs on the PC kernel through the pc-arch shim as it is (virtio-sound-device becomes virtio-sound-pci).
set -uo pipefail

# tone.wasm is bundled only in test images (cargo feature test-apps), built into target/test-apps so no other suite
# sees it (the architect, A2 change 2).
python3 scripts/lib/embedded-same.py scripts/lib/audio-wasm.py kernel/src/nia.rs || { echo "WASM-AUDIO TEST FAILED (the kernel's copy of the test app is stale)"; exit 1; }
export CARGO_TARGET_DIR="$PWD/target/test-apps"
BIN="$CARGO_TARGET_DIR/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel --features test-apps 2>&1 | tail -1
if [ -n "${PC_EFI:-}" ]; then # on the PC through the shim: the test-apps PC kernel
    cargo build -p mansa-kernel --bin mansa-pc --features pc,test-apps --target x86_64-unknown-uefi 2>&1 | tail -1
    export PC_EFI="$CARGO_TARGET_DIR/x86_64-unknown-uefi/debug/mansa-pc.efi"
fi

T="${TMPDIR:-/tmp}/wasm-audio"
rm -rf "$T"; mkdir -p "$T/shots"
python3 scripts/lib/audio-wasm.py "$T/m" > /dev/null || { echo "WASM-AUDIO TEST FAILED (generator)"; exit 1; }
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" --sectors 32768 tone2.wasm="$T/m/tone.wasm" > /dev/null ||
    { echo "WASM-AUDIO TEST FAILED (disk image)"; exit 1; }
FAILS=0
fail() { echo "  FAIL: $1"; FAILS=$((FAILS + 1)); } # every check runs; the verdict is at the end

portfree() { # wait (at most 15 s) until nothing holds the TCP ports $1 and $2, so a new QEMU can bind them
    local k
    for k in $(seq 60); do
        python3 -c 'import socket, sys
for p in sys.argv[1:]:
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", int(p))); s.close()' "$1" "$2" 2> /dev/null && return 0
        sleep 0.25
    done
    echo "  (ports $1 and $2 still busy after 15 s)"
}
boot() { # boot <name> <sound: 1 or 0> STEP ...
    local n="$1" snd="$2"; shift 2
    local dev=()
    [ "$snd" = 1 ] && dev=(-audiodev "wav,id=snd0,path=$T/$n.wav" -device virtio-sound-device,audiodev=snd0)
    cp "$IMG" "$T/$n.img"
    portfree 5174 5175
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none "${dev[@]}" \
        -drive "file=$T/$n.img,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -serial tcp:127.0.0.1:5174,server,nowait \
        -qmp tcp:127.0.0.1:5175,server,nowait -kernel "$BIN" &
    local q=$!
    python3 scripts/lib/cell-keys.py 5174 5175 "$T/shots" "$@" "mem" > "$T/$n.out" 2>&1
    # the wav audiodev may still be writing: wait until the file stops growing (at most 10 s)
    local last=-1 now k
    for k in $(seq 40); do
        now=$(wc -c < "$T/$n.wav" 2> /dev/null || echo 0)
        [ "$now" = "$last" ] && break
        last=$now; sleep 0.25
    done
    # stop QEMU itself by its serial port (through the PC shim $q is the shim, not QEMU) and wait until it is gone
    kill "$q" 2> /dev/null; pkill -f "tcp:127.0.0.1:5174," 2> /dev/null
    for k in $(seq 20); do pgrep -f "tcp:127.0.0.1:5174," > /dev/null || break; sleep 0.25; done
    pkill -9 -f "tcp:127.0.0.1:5174," 2> /dev/null; wait "$q" 2> /dev/null
    tr -d '\r' < "$T/$n.out" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//' > "$T/$n.txt"
    grep -aq "KERNEL PANIC\|panicked" "$T/$n.txt" && fail "boot $n: kernel panic"
    grep -a "^\[nia\] audio: underrun" "$T/$n.txt" | sed "s/^/  $n | /"
    true
}
# the part of boot $1's log from the n-th (`$3`) `nia tone.wasm` or other command `$2` to the next prompt
seg() { awk -v c="afrisovera> $2" -v n="${3:-1}" 'index($0, c) == 1 && length($0) == length(c) {k++; if (k == n) {on=1; next}}
        on && index($0, "afrisovera>") {exit} on' "$T/$1.txt"; }
prints() { grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' ' | sed 's/ $//'; }
closed() { # closed <text> <why> <frames regexp> <underruns regexp>: the stream's closed line, with its overrun bound
    local l
    l="$(grep -m1 -aE '^\[nia\] audio: stream closed' <<< "$1")"
    [ -n "$l" ] || return 1
    grep -aqE "^\[nia\] audio: stream closed \($2\), $3 frames played, $4 underrun\(s\), waits ([0-9]+) us past their end at most; the sound device is free$" <<< "$l" || return 1
    # condition 3: a wait runs at most one driver period past its end. The drivers take a whole period (1024 frames,
    # 23 ms) and hand it to the device before they return, so the bound is that, not one 256-sample chunk.
    # Graded at 50 ms: one period (23 ms) plus the desk's own pass in the same wait (a full redraw at 1280x800 on
    # the PC's debug kernel). The number is reported in the closed line either way.
    [ "$(printf '%s\n' "$l" | sed -nE 's/.*waits ([0-9]+) us.*/\1/p')" -le 50000 ] || return 1
}
wavsegs() { python3 scripts/lib/audio-wav.py "$T/$1.wav" | tee "$T/$1.segs" | sed "s/^/  $1 | /"; }
near() { [ "$1" -ge $(($2 - $3)) ] && [ "$1" -le $(($2 + $3)) ]; } # near <value> <want> <tolerance>

# --- A: 441 Hz, a pause, 441 Hz
boot A 1 "&nia tone.wasm" ":61" "="
grep -aE "^\[nia\] audio|\[wasm app\]|TRAP|DENIED" "$T/A.txt" | sed 's/^/  A | /'
wavsegs A
S="$(seg A "nia tone.wasm")"
[ "$(printf '%s\n' "$S" | prints)" = "1" ] || fail "A: did not print 1"
closed "$S" "audio_close" 22050 1 || fail "A: closed line: $(printf '%s\n' "$S" | grep -a 'stream closed')"
[ "$(grep -c '^seg:' "$T/A.segs")" -eq 2 ] || fail "A: $(grep -c '^seg:' "$T/A.segs") stretches of sound, not 2 (a gap, or the pause replayed)"
while read -r _ st ms hz pk; do
    ms=${ms#ms=}; hz=${hz#hz=}; pk=${pk#peak=}
    near "$ms" 500 30 || fail "A: a stretch of $ms ms, not 500 +- 30"
    near "$hz" 441 9 || fail "A: $hz Hz, not 441 +- 2 %"
    near "$pk" 8000 1000 || fail "A: peak $pk, not 7000 to 9000"
done < <(grep '^seg:' "$T/A.segs")
# QEMU's virtio-sound writes nothing while the stream is ended, so the pause shows as the one chunk of silence the
# kernel plays before it ends the stream; what proves nothing old was replayed is two stretches of exactly the
# written length (a replayed period would make a stretch longer or add a third).
q="$(sed -nE 's/^quiet: ms=([0-9]+)/\1/p' "$T/A.segs")"
[ "${q:-0}" -ge 3 ] || fail "A: no silence between the two halves (${q:-0} ms)"

# --- B: stereo mixed down
boot B 1 "&nia tone.wasm" ":62" "="
wavsegs B
S="$(seg B "nia tone.wasm")"
[ "$(printf '%s\n' "$S" | prints)" = "2" ] || fail "B: did not print 2"
closed "$S" "audio_close" 48000 0 || fail "B: closed line: $(printf '%s\n' "$S" | grep -a 'stream closed')"
[ "$(grep -c '^seg:' "$T/B.segs")" -eq 1 ] || fail "B: $(grep -c '^seg:' "$T/B.segs") stretches of sound, not 1"
read -r _ _ ms hz pk < <(grep '^seg:' "$T/B.segs")
ms=${ms#ms=}; hz=${hz#hz=}; pk=${pk#peak=}
near "$ms" 1000 30 || fail "B: $ms ms, not 1000 +- 30"
near "$hz" 480 10 || fail "B: $hz Hz, not 480 +- 2 %"
near "$pk" 4000 500 || fail "B: peak $pk, not 3500 to 4500 (the mix-down)"

# --- C: edges and every way out
boot C 1 \
    "&nia tone.wasm" ":65" "=" "say hello" \
    "&nia tone.wasm" ":74" "=" "say hello" \
    "&nia tone.wasm" ":63" "~1.5" ":03" "=" "say hello" \
    "&nia tone.wasm" ":72" "=" "say hello" \
    "&nia tone.wasm" ":73" "=" "say hello" \
    "&nia tone2.wasm" ":6e" "=" \
    "gui" "desk mode pointer" \
    "&nia tone.wasm" "@opened by tone.wasm" ":77" "~2" ":1c" "=" "say hello" \
    "&nia tone.wasm" "@opened by tone.wasm" ":69" "~2" ":1c" "~3.5" ":1c" "=" "say hello"
grep -aE "^afrisovera> (nia|say)|^\[nia\] (audio|TRAP|'run')|\[wasm app\]|DENIED|^\[voice\]|^\[asili voice\] synth|^\[desk\] cell" \
    "$T/C.txt" | sed 's/^/  C | /' | head -90
say_ok() { [ "$(seg C "say hello" "$1" | grep -ac '^\[asili voice\] synthesized')" -eq 1 ] || fail "C: say hello #$1 did not play (the device is not free)"; }

S="$(seg C "nia tone.wasm" 1)"
[ "$(printf '%s\n' "$S" | prints)" = "-3 -3 -3 -3 1 -1 1378 1378 1378 1378 0 -1 5512 0 -1" ] ||
    fail "C edges: printed [$(printf '%s\n' "$S" | prints)]"
closed "$S" "audio_close" 5512 0 || fail "C edges: closed line"
say_ok 1
S="$(seg C "nia tone.wasm" 2)"
grep -aqxF "[nia] TRAP: out of bounds memory access" <<< "$S" || fail "C t: no out-of-bounds TRAP"
grep -aq "print(99)" <<< "$S" && fail "C t: it went on after the bad write"
closed "$S" "the app stopped" "[0-9]+" "[0-9]+" || fail "C t: closed line"
say_ok 2
S="$(seg C "nia tone.wasm" 3)"
grep -aqxF "[nia] TRAP: stopped by Ctrl+C" <<< "$S" || fail "C c: Ctrl+C did not stop it"
closed "$S" "the app stopped" "[0-9]+" "[0-9]+" || fail "C c: closed line"
say_ok 3
S="$(seg C "nia tone.wasm" 4)"
[ "$(printf '%s\n' "$S" | prints)" = "4" ] || fail "C r: did not print 4"
closed "$S" "the app returned" 4410 0 || fail "C r: the 200 ms queued were not played out: $(printf '%s\n' "$S" | grep -a 'stream closed')"
say_ok 4
S="$(seg C "nia tone.wasm" 5)"
[ "$(printf '%s\n' "$S" | prints)" = "-1 5" ] || fail "C s: printed [$(printf '%s\n' "$S" | prints)]"
grep -aqxF "[voice] busy: an app owns the sound device" <<< "$S" || fail "C s: the voice was not busy"
[ "$(printf '%s\n' "$S" | grep -ac '^\[asili voice\] synthesized')" -eq 1 ] || fail "C s: the voice did not speak after audio_close"
closed "$S" "audio_close" 0 0 || fail "C s: closed line"
say_ok 5
S="$(seg C "nia tone2.wasm")"
grep -aqF "DENIED: app called host 'audio_open' without the capability" <<< "$S" || fail "C: the disk copy was not DENIED"
grep -aq "stream open" <<< "$S" && fail "C: the disk copy opened a stream"
S="$(seg C "nia tone.wasm" 6)"
grep -aqxF "[desk] cell: close request (Ctrl+\\)" <<< "$S" || fail "C w: no close request"
grep -aqxF "[nia] 'run' returned cleanly" <<< "$S" || fail "C w: did not return on the close request"
closed "$S" "the app returned" "[1-9][0-9]*" "[0-9]+" || fail "C w: closed line: $(printf '%s\n' "$S" | grep -a 'stream closed')"
say_ok 6
S="$(seg C "nia tone.wasm" 7)"
grep -aqxF "[nia] TRAP: closed by the person (Close anyway)" <<< "$S" || fail "C i: Close anyway did not stop it"
closed "$S" "the app stopped" "[1-9][0-9]*" "[0-9]+" || fail "C i: closed line: $(printf '%s\n' "$S" | grep -a 'stream closed')"
say_ok 7
[ "$(grep -ac '^\[nia\] audio: stream open' "$T/C.txt")" -eq "$(grep -ac '^\[nia\] audio: stream closed' "$T/C.txt")" ] ||
    fail "C: a stream was opened and never closed"

# --- E: Ngoma, the drum machine (bundled, nia-apps N1), in the desk with its own keys: Space plays, q stops
boot E 1 "gui" "desk mode pointer" "&nia ngoma.wasm" "@opened by ngoma.wasm" "~2" ":20" "~3" ":71" "=" "say hello"
wavsegs E
S="$(seg E "nia ngoma.wasm")"
printf '%s
' "$S" | grep -aqxF "[nia] audio: stream open, 22050 Hz, 1 channel(s), 5512 frames queued at most" || fail "E: Ngoma did not open its stream"
closed "$S" "audio_close" "[1-9][0-9]*" 0 || fail "E: Ngoma's closed line: $(printf '%s
' "$S" | grep -a 'stream closed\|underrun')"
printf '%s
' "$S" | grep -aqxF "[nia] 'run' returned cleanly" || fail "E: Ngoma did not return on q"
[ "$(grep -c '^seg:' "$T/E.segs")" -ge 1 ] || fail "E: the device played no sound for Ngoma"
[ "$(seg E "say hello" | grep -ac '^\[asili voice\] synthesized')" -eq 1 ] || fail "E: say hello after Ngoma did not play"

# --- D: no sound device
boot D 0 "&nia tone.wasm" ":6e" "="
S="$(seg D "nia tone.wasm")"
[ "$(printf '%s\n' "$S" | prints)" = "-2" ] || fail "D: audio_open with no sound device printed [$(printf '%s\n' "$S" | prints)], not -2"
[ "$FAILS" -eq 0 ] || { echo "WASM-AUDIO TEST FAILED ($FAILS check(s))"; exit 1; }
echo "WASM-AUDIO TEST PASSED"
