#!/usr/bin/env bash
# wasm-oob (claude/nia-addr): memory addresses as WebAssembly says. scripts/lib/oob-wasm.py writes 11 small
# modules (a base of -1, the last bytes of 64 KiB for 1-, 2- and 8-byte accesses, the offset immediate, data
# segments at the edge) and what each must do; the suite boots them from a disk image and runs each with
# `nia NAME.wasm`. Graded per module: a TRAP "out of bounds memory access" with print(99) never reached, or the
# exact value then print(99), or the parse error for a segment past the end. No panic; the shell answers.
# Every case matches a standard WebAssembly engine. Then `wasmoob` (a built-in full-capability app) calls speak
# with a pointer of -1: TRAP. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-oob"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/oob-wasm.py "$T/m" > "$T/spec.txt" || { echo "WASM-OOB TEST FAILED (generator)"; exit 1; }
ARGS=(); CMDS=()
while read -r n _; do ARGS+=("$n.wasm=$T/m/$n.wasm"); CMDS+=("nia $n.wasm"); done < "$T/spec.txt"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-OOB TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4806,server,nowait \
    -qmp tcp:127.0.0.1:4807,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="$T/out.txt"
AFRISH_CMD_QUIET="${AFRISH_CMD_QUIET:-4}" \
python3 scripts/shell-demo.py 4806 4807 "$T/shot.png" "${CMDS[@]}" "wasmoob" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

tr -d '\r' < "$OUT" > "$T/clean.txt"
grep -aE "afrisovera> nia oob|\[wasm app\]|TRAP|parse error" "$T/clean.txt" | sed 's/^/  /' | head -60
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && { echo "WASM-OOB TEST FAILED (kernel panic)"; exit 1; }

bad=0
while read -r n kind expect; do
    # this module's part of the log: from its command line to the next shell command
    seg="$(awk -v n="$n" 'index($0, "afrisovera>") && $0 ~ ("nia " n "[.]wasm[ 	]*$") {on=1; next}
        on && index($0, "afrisovera>") {exit} on' "$T/clean.txt")"
    case "$kind" in
        trap)
            grep -aqF "[nia] TRAP: out of bounds memory access" <<< "$seg" && ! grep -aqF "print(99)" <<< "$seg" \
                || { echo "  $n: expected a TRAP (out of bounds) before print(99)"; bad=1; } ;;
        value)
            grep -aqF "[wasm app] $expect" <<< "$seg" && grep -aqF "print(99)" <<< "$seg" \
                || { echo "  $n: expected $expect then print(99)"; bad=1; } ;;
        parse)
            grep -aqF "[nia] parse error: $expect" <<< "$seg" \
                || { echo "  $n: expected the parse error '$expect'"; bad=1; } ;;
        runs)
            grep -aqF "print(99)" <<< "$seg" && ! grep -aqF "TRAP" <<< "$seg" \
                || { echo "  $n: expected to run to print(99) with no TRAP"; bad=1; } ;;
    esac
done < "$T/spec.txt"
# The host-pointer case (a full-capability built-in app, `wasmoob`): speak with a pointer of -1 must TRAP.
seg="$(awk 'index($0, "afrisovera>") && $0 ~ /wasmoob[ 	]*$/ {on=1; next} on && index($0, "afrisovera>") {exit} on' "$T/clean.txt")"
grep -aqF "[nia] TRAP: out of bounds memory access" <<< "$seg" && ! grep -aqF "print(99)" <<< "$seg"     || { echo "  wasmoob: speak(-1, 10) did not TRAP (out of bounds) before print(99)"; bad=1; }
[ "$bad" -eq 0 ] || { echo "WASM-OOB TEST FAILED (see the modules above)"; exit 1; }
grep -aq "heap:" "$T/clean.txt" || { echo "WASM-OOB TEST FAILED (the shell stopped answering)"; exit 1; }
echo "WASM-OOB TEST PASSED  ($(wc -l < "$T/spec.txt") modules and wasmoob: TRAP past the end and at -1, values at the last bytes, the offset immediate, data segments at the edge and a giant length, draw arguments of -1, a host pointer of -1)"
