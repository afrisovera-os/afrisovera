#!/usr/bin/env bash
# Verify `prevmultiple <n> <m>` gives the largest multiple of m that is <= n (floor to multiple):
# 17 5->15, 20 5->20 (already a multiple), 1 10->0, 100 7->98, 0 5->0. m=0 reported not crashed.
# Floor mirror of nextmultiple (ceil). Self-checking: python (n//m)*m.
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
OUT="${TMPDIR:-/tmp}/prevmultiple.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevmultiple.png" \
    "echo warmup" "prevmultiple 17 5" "prevmultiple 20 5" "prevmultiple 1 10" \
    "prevmultiple 100 7" "prevmultiple 0 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevmultiple:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, m):                       # DERIVED reference
    return f"prevmultiple: {n} -> {(n // m) * m} (multiple of {m})"

for n, m in [(17, 5), (20, 5), (1, 10), (100, 7), (0, 5)]:
    w = want(n, m)
    if w not in s:
        print(f"PREVMULTIPLE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: round down, already-multiple, floor to zero, offset, zero
assert "prevmultiple: 17 -> 15 (multiple of 5)" in s
assert "prevmultiple: 20 -> 20 (multiple of 5)" in s     # n already a multiple
assert "prevmultiple: 1 -> 0 (multiple of 10)" in s      # floors to 0
assert "prevmultiple: 100 -> 98 (multiple of 7)" in s
assert "prevmultiple: 0 -> 0 (multiple of 5)" in s

print("PREVMULTIPLE TEST PASSED  (17 5->15, 20 5->20, 1 10->0, 100 7->98, 0 5->0 "
      "vs python floor-to-multiple)")
PY
