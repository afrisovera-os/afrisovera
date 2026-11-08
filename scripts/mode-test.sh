#!/usr/bin/env bash
# Verify the `mode` command reports the most-frequent value of an integer list,
# with the SMALLEST value winning on ties: 1 2 2 3 -> 2 (x2); 5 5 1 1 1 -> 1 (x3);
# 7 7 3 3 -> 3 (tie at 2 each -> smallest). Test recomputes the mode in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4944,server,nowait \
    -qmp tcp:127.0.0.1:4945,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/mode.out"
python3 scripts/shell-demo.py 4944 4945 "${TMPDIR:-/tmp}/mode.png" \
    "mode 1 2 2 3" "mode 5 5 1 1 1" "mode 7 7 3 3" "mode" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mode:|usage: mode" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def mode(a):
    a = sorted(a); best_v = a[0]; best_c = 0; cv = a[0]; cc = 0
    for x in a:
        if x == cv: cc += 1
        else: cv = x; cc = 1
        if cc > best_c: best_c = cc; best_v = cv
    return best_v, best_c

for lst in [[1, 2, 2, 3], [5, 5, 1, 1, 1], [7, 7, 3, 3]]:
    v, c = mode(lst)
    want = f"mode: {v} (appears {c} times of {len(lst)} values)"
    if want not in s:
        print(f"MODE TEST FAILED (expected '{want}' for {lst})"); sys.exit(1)

if "usage: mode <n1> <n2> ..." not in s:
    print("MODE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("MODE TEST PASSED  (1 2 2 3->2x2, 5 5 1 1 1->1x3, 7 7 3 3->3 smallest-on-tie; empty->usage)")
PY
