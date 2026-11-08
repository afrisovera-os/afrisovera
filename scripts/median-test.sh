#!/usr/bin/env bash
# Verify the `median` command computes the median of a list of integers (parse ->
# sort -> middle). Odd count -> the middle element; even count -> the integer floor
# of the average of the two central values. 3 1 2 -> 2; 1 2 3 4 -> 2; 10..50 -> 30.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4900,server,nowait \
    -qmp tcp:127.0.0.1:4901,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/median.out"
python3 scripts/shell-demo.py 4900 4901 "${TMPDIR:-/tmp}/median.png" \
    "median 3 1 2" "median 1 2 3 4" "median 10 20 30 40 50" "median" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "median:|usage: median" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def med(a):
    a = sorted(a); n = len(a)
    return a[n // 2] if n % 2 else (a[n // 2 - 1] + a[n // 2]) // 2

cases = [[3, 1, 2], [1, 2, 3, 4], [10, 20, 30, 40, 50]]
for lst in cases:
    want = f"median: {med(lst)} (of {len(lst)} values)"
    if want not in s:
        print(f"MEDIAN TEST FAILED (expected '{want}' for {lst})"); sys.exit(1)

if "usage: median <n1> <n2> ..." not in s:
    print("MEDIAN TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("MEDIAN TEST PASSED  (3 1 2->2, 1 2 3 4->2 floor, 10..50->30; empty->usage)")
PY
