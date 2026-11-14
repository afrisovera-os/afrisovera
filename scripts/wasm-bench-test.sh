#!/usr/bin/env bash
# Nia speed step 1 (claude/nia-fast): `wasmbench` times two bench apps the old way (a scan to each block's end
# on every entry) and the new way (the block-end memo), in the same kernel. The suite checks that both ways
# print the right value (scripts/lib/bench-wasm.py --expect) and that the new way is not slower. The times
# themselves are evidence for the plan, not a pass mark: QEMU timing moves with the cloud machine's load.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4624,server,nowait \
    -qmp tcp:127.0.0.1:4625,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-bench.out"
python3 scripts/lib/nia-keys.py 4624 "wasmbench" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[bench\]|print\(|TRAP" "$OUT" | sed 's/^/  /'

fail() { echo "WASM-BENCH TEST FAILED ($1)"; exit 1; }
grep -aq "KERNEL PANIC" "$OUT" && fail "kernel panic"
grep -aq "\[bench\] done" "$OUT" || fail "wasmbench did not finish"
grep -aq "TRAP" "$OUT" && fail "a bench app trapped"
# Three runs the old way and three the new way, each printing the right value.
B1=$(grep -ac "print(200017500)" "$OUT")
B2=$(grep -ac "print(12502500)" "$OUT")
[ "$B1" -eq 6 ] || fail "b1 printed 200017500 $B1 times, wanted 6"
[ "$B2" -eq 6 ] || fail "b2 printed 12502500 $B2 times, wanted 6"
grep -aq "heap:" "$OUT" || fail "shell unresponsive after wasmbench"
# The new way must not be slower than the old way (each app's best of three).
while read -r old new; do
    [ "$new" -le "$old" ] || fail "new way slower: old $old us, new $new us"
done < <(grep -a "\[bench\] b" "$OUT" | sed -E 's/.*old ([0-9]+) us, new ([0-9]+) us.*/\1 \2/')
echo "WASM-BENCH TEST PASSED (both bench apps right both ways; block-end memo not slower)"
