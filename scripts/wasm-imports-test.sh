#!/usr/bin/env bash
# wasm-imports (claude/nia-imports): a module may import 16 functions instead of 8. scripts/lib/imports-wasm.py
# writes three apps; the suite boots them from a disk image (unverified apps) and drives them with
# scripts/lib/poll-keys.py. Graded: im16 (16 imports, print in the last slot) prints 16 then 99; im17 (17 imports)
# is refused at load with "(Nia holds 16)"; clock9 (9 imports: the clock a window app needs) loads, ticks every
# 200 ms on sleep_ms until a key, and prints 3 or more ticks, then time(), which must be 0 to 15 s after the
# `date` typed just before it (clock9 calls `time` since OS_REQUESTS item 7). No TRAP, no DENIED, no panic; the
# shell answers.
# Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-imports"
rm -rf "$T"; mkdir -p "$T"
python3 scripts/lib/imports-wasm.py "$T/m" > "$T/names.txt" || { echo "WASM-IMPORTS TEST FAILED (generator)"; exit 1; }
ARGS=()
while read -r n _; do ARGS+=("$n.wasm=$T/m/$n.wasm"); done < "$T/names.txt"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-IMPORTS TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4836,server,nowait \
    -qmp tcp:127.0.0.1:4837,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/poll-keys.py 4836 \
    "nia im16.wasm" \
    "nia im17.wasm" \
    "date" "&nia clock9.wasm" "~1.2" ":20" "=" \
    "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

tr -d '\r' < "$OUT" > "$T/clean.txt"
grep -aE "afrisovera> nia |\[wasm app\]|TRAP|parse error|DENIED|poll-keys:" "$T/clean.txt" | sed 's/^/  /' | head -40
fail() { echo "WASM-IMPORTS TEST FAILED ($1)"; exit 1; }
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
grep -aq "DENIED: app called host" "$T/clean.txt" && fail "a host function was denied"
grep -aq "TRAP" "$T/clean.txt" && fail "an app trapped"

seg() {
    awk -v n="$1" 'index($0, "afrisovera>") && $0 ~ ("nia " n "[.]wasm[ \t]*$") {on=1; next}
        on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"
}
prints() { seg "$1" | grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' '; }

[ "$(prints im16)" = "16 99 " ] || fail "im16: expected 16 then 99 from the 16th import, got '$(prints im16)'"
grep -aq "parse error" <<< "$(seg im16)" && fail "im16: 16 imports were refused"
grep -aqF "[nia] parse error: too many imported functions (Nia holds 16)" <<< "$(seg im17)" \
    || fail "im17: 17 imports were not refused with '(Nia holds 16)'"
grep -aq "parse error" <<< "$(seg clock9)" && fail "clock9: the 9-import app was refused"
read -r t e _ <<< "$(prints clock9)"
[ -n "${t:-}" ] && [ "$t" -ge 3 ] || fail "clock9: expected 3 or more ticks, got '${t:-}'"
d="$(grep -aoE '\(epoch [0-9]+\)' "$T/clean.txt" | head -1 | tr -dc '0-9')"
[ -n "$d" ] && [ -n "${e:-}" ] && [ $((e - d)) -ge 0 ] && [ $((e - d)) -le 15 ] \
    || fail "clock9: time() '${e:-}' is not 0 to 15 s after date '$d'"
grep -aq "heap:" "$T/clean.txt" || fail "the shell stopped answering"
echo "WASM-IMPORTS TEST PASSED  (16 imports load and the 16th is called; 17 refused; a 9-import clock ticked $t times and read the time $((e - d)) s after date)"
