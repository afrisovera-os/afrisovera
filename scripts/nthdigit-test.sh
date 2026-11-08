#!/usr/bin/env bash
# Verify `nthdigit <k> <n>` returns the kth digit (1-based, from the MOST-significant/left) of n:
# 1 12345->1, 3 12345->3, 5 12345->5, 6 12345->(out of range), 1 7->7. Digit-extraction primitive
# vs maxdigit/digitcount/digitsum. Self-checking: python str(n)[k-1] if 1<=k<=len(str(n)).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/nthdigit.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthdigit.png" \
    "echo warmup" "nthdigit 1 12345" "nthdigit 3 12345" "nthdigit 5 12345" \
    "nthdigit 6 12345" "nthdigit 1 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthdigit:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(k, n):                       # DERIVED reference
    ds = str(n)
    return "nthdigit: " + (ds[k - 1] if 1 <= k <= len(ds) else "(out of range)")

for k, n in [(1, 12345), (3, 12345), (5, 12345), (6, 12345), (1, 7)]:
    w = want(k, n)
    if w not in s:
        print(f"NTHDIGIT TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: first, middle, last, past-end, single-digit
assert "nthdigit: 1" in s
assert "nthdigit: 3" in s
assert "nthdigit: 5" in s
assert "nthdigit: (out of range)" in s
assert "nthdigit: 7" in s

print("NTHDIGIT TEST PASSED  (1 12345->1, 3 12345->3, 5 12345->5, 6 12345->(out of range), "
      "1 7->7 vs python str(n)[k-1])")
PY
