#!/usr/bin/env bash
# Verify `nextmultiple <n> <m>` gives the smallest multiple of m that is >= n (ceil to multiple):
# 17 5->20, 20 5->20 (already a multiple), 1 10->10, 100 7->105, 0 5->0. m=0 reported not crashed.
# Self-checking: python ((n+m-1)//m)*m.
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
OUT="${TMPDIR:-/tmp}/nextmultiple.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextmultiple.png" \
    "echo warmup" "nextmultiple 17 5" "nextmultiple 20 5" "nextmultiple 1 10" \
    "nextmultiple 100 7" "nextmultiple 0 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextmultiple:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, m):                       # DERIVED reference
    up = ((n + m - 1) // m) * m
    return f"nextmultiple: {n} -> {up} (multiple of {m})"

for n, m in [(17, 5), (20, 5), (1, 10), (100, 7), (0, 5)]:
    w = want(n, m)
    if w not in s:
        print(f"NEXTMULTIPLE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: round up, already-multiple, small n, offset, zero
assert "nextmultiple: 17 -> 20 (multiple of 5)" in s
assert "nextmultiple: 20 -> 20 (multiple of 5)" in s     # n already a multiple
assert "nextmultiple: 1 -> 10 (multiple of 10)" in s
assert "nextmultiple: 100 -> 105 (multiple of 7)" in s
assert "nextmultiple: 0 -> 0 (multiple of 5)" in s       # zero

print("NEXTMULTIPLE TEST PASSED  (17 5->20, 20 5->20, 1 10->10, 100 7->105, 0 5->0 "
      "vs python ceil-to-multiple)")
PY
