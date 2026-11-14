#!/usr/bin/env bash
# wasm-textrun (claude/nia-a1a3, OS_REQUESTS item 10 A1 and A3): a module may have 128 functions and 32 types, and
# win_text_run draws one row of text in one call. scripts/lib/textrun-wasm.py writes the apps (read it for each
# one); the suite boots them from a disk image (unverified apps: print, draw and input are granted) and drives
# them with scripts/lib/cell-keys.py, which also takes the screenshots. Graded:
#   f128 (128 functions, 32 types; run calls each of the other 127 and the print import) prints the generator's
#   sum and returns; f129 and t33 are refused at load with "(Nia holds 128)" and "(Nia holds 32)";
#   tr1 (one win_text_run) and tr2 (eight win_text calls) give the same picture, pixel for pixel, and it is not
#   tr0's (the window without text), so text was drawn;
#   tr3 (len 0, len 1000 at 65500 cut to 30 before the memory check, x = -16, window 9) prints 2 3 4 5 1 and
#   returns; tr4 (65530 + 30, past the 64 KiB) ends in "TRAP: out of bounds memory access" with no print.
# No other TRAP, no DENIED, no panic; the shell answers `mem` at the end. Runs on the PC kernel through the
# pc-arch shim as it is. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}/wasm-textrun"
rm -rf "$T"; mkdir -p "$T/shots"
python3 scripts/lib/textrun-wasm.py "$T/m" > "$T/names.txt" || { echo "WASM-TEXTRUN TEST FAILED (generator)"; exit 1; }
ARGS=()
while read -r n _; do ARGS+=("$n.wasm=$T/m/$n.wasm"); done < "$T/names.txt"
SUM="$(sed -nE 's/^f128 print\(([0-9]+)\).*/\1/p' "$T/names.txt")"
IMG="$T/disk.img"
python3 scripts/lib/mkvfs-big.py image "$IMG" "${ARGS[@]}" > /dev/null || { echo "WASM-TEXTRUN TEST FAILED (disk image)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4892,server,nowait \
    -qmp tcp:127.0.0.1:4893,server,nowait -kernel "$BIN" &
QPID=$!
OUT="$T/out.txt"
python3 scripts/lib/cell-keys.py 4892 4893 "$T/shots" \
    "nia f128.wasm" "nia f129.wasm" "nia t33.wasm" \
    "&nia tr0.wasm" "@print(1)" "~1" "!tr0" ":71" "=" \
    "&nia tr1.wasm" "@print(1)" "~1" "!tr1" ":71" "=" \
    "&nia tr2.wasm" "@print(1)" "~1" "!tr2" ":71" "=" \
    "nia tr3.wasm" "nia tr4.wasm" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

tr -d '\r' < "$OUT" > "$T/clean.txt"
grep -aE "afrisovera> nia |\[wasm app\]|TRAP|parse error|DENIED|returned cleanly|cell-keys:" "$T/clean.txt" | sed 's/^/  /' | head -60
fail() { echo "WASM-TEXTRUN TEST FAILED ($1)"; exit 1; }
grep -aq "KERNEL PANIC\|panicked" "$T/clean.txt" && fail "kernel panic"
grep -aq "DENIED: app called host" "$T/clean.txt" && fail "a host function was denied"
[ -n "$SUM" ] || fail "harness: no f128 sum from the generator"

seg() {
    awk -v n="$1" 'index($0, "afrisovera>") && $0 ~ ("nia " n "[.]wasm[ \t]*$") {on=1; next}
        on && index($0, "afrisovera>") {exit} on' "$T/clean.txt"
}
prints() { seg "$1" | grep -aoE '\[wasm app\] print\(-?[0-9]+\)' | grep -oE '[-]?[0-9]+' | tr '\n' ' '; }
clean() { seg "$1" | grep -aqE '\[nia\] .run. returned cleanly' || fail "$1: did not return cleanly"; }
notrap() { seg "$1" | grep -aq "TRAP\|parse error" && fail "$1: $(seg "$1" | grep -a 'TRAP\|parse error' | head -1)"; true; }

[ "$(prints f128)" = "$SUM " ] || fail "f128: expected print($SUM), got '$(prints f128)'"
notrap f128; clean f128
grep -aqF "[nia] parse error: too many functions (Nia holds 128)" <<< "$(seg f129)" \
    || fail "f129: 129 functions were not refused with '(Nia holds 128)'"
grep -aqF "[nia] parse error: too many types (Nia holds 32)" <<< "$(seg t33)" \
    || fail "t33: 33 types were not refused with '(Nia holds 32)'"
for n in tr0 tr1 tr2; do
    [ "$(prints $n)" = "1 " ] || fail "$n: expected print(1), got '$(prints $n)'"
    notrap $n; clean $n
    [ -s "$T/shots/$n.png" ] || fail "$n: no screenshot"
done
same12="$(python3 scripts/lib/png-same.py "$T/shots/tr1.png" "$T/shots/tr2.png")"
same10="$(python3 scripts/lib/png-same.py "$T/shots/tr1.png" "$T/shots/tr0.png")"
echo "  tr1 against tr2: $same12"
echo "  tr1 against tr0: $same10"
case "$same12" in same*) ;; *) fail "tr1 (win_text_run) and tr2 (eight win_text) differ: $same12" ;; esac
case "$same10" in differ:*) ;; *) fail "tr1 is the same as tr0 (no text drawn?): $same10" ;; esac
[ "$(prints tr3)" = "2 3 4 5 1 " ] || fail "tr3: expected 2 3 4 5 1, got '$(prints tr3)'"
notrap tr3; clean tr3
[ -z "$(prints tr4)" ] || fail "tr4: printed '$(prints tr4)' before its TRAP"
seg tr4 | grep -aq "TRAP: out of bounds memory access" || fail "tr4: a run past the 64 KiB did not TRAP"
grep -aq "heap:" "$T/clean.txt" || fail "the shell did not answer mem at the end"
echo "WASM-TEXTRUN TEST PASSED (128 functions and 32 types load, 129 and 33 are refused; win_text_run draws what eight win_text calls draw, cuts at 30, traps past the memory)"
