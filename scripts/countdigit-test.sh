#!/usr/bin/env bash
# Verify `countdigit <d> <n>` counts occurrences of digit d in n: 1 12131->3, 0 1000->3,
# 5 12345->1, 9 12345->0, 0 0->1. Distinct from digitcount (total)/maxdigit/digitsum.
# Self-checking: python str(n).count(str(d)).
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
OUT="${TMPDIR:-/tmp}/countdigit.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countdigit.png" \
    "echo warmup" "countdigit 1 12131" "countdigit 0 1000" "countdigit 5 12345" \
    "countdigit 9 12345" "countdigit 0 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countdigit:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(d, n):                       # DERIVED reference
    return f"countdigit: {d} occurs {str(n).count(str(d))} times in {n}"

for d, n in [(1, 12131), (0, 1000), (5, 12345), (9, 12345), (0, 0)]:
    w = want(d, n)
    if w not in s:
        print(f"COUNTDIGIT TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: repeated, trailing zeros, single, absent (0), n=0 single digit
assert "countdigit: 1 occurs 3 times in 12131" in s
assert "countdigit: 0 occurs 3 times in 1000" in s
assert "countdigit: 5 occurs 1 times in 12345" in s
assert "countdigit: 9 occurs 0 times in 12345" in s     # absent
assert "countdigit: 0 occurs 1 times in 0" in s          # n=0 is one '0'

print("COUNTDIGIT TEST PASSED  (1 12131->3, 0 1000->3, 5 12345->1, 9 12345->0, 0 0->1 "
      "vs python str(n).count(str(d)))")
PY
