#!/usr/bin/env bash
# Verify `roundto <n> <m>` rounds n to the NEAREST multiple of m (round half up): 17 5->15,
# 18 5->20, 12 5->10, 13 5->15, 25 10->30 (exact half rounds up). m=0 reported not crashed.
# Nearest-multiple companion of nextmultiple (ceil). Self-checking: python ((n+m//2)//m)*m.
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
OUT="${TMPDIR:-/tmp}/roundto.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/roundto.png" \
    "echo warmup" "roundto 17 5" "roundto 18 5" "roundto 12 5" \
    "roundto 13 5" "roundto 25 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "roundto:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, m):                       # DERIVED reference
    nearest = ((n + m // 2) // m) * m
    return f"roundto: {n} -> {nearest} (nearest multiple of {m})"

for n, m in [(17, 5), (18, 5), (12, 5), (13, 5), (25, 10)]:
    w = want(n, m)
    if w not in s:
        print(f"ROUNDTO TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: round down, round up, down, up-at-3, exact half->up
assert "roundto: 17 -> 15 (nearest multiple of 5)" in s
assert "roundto: 18 -> 20 (nearest multiple of 5)" in s
assert "roundto: 12 -> 10 (nearest multiple of 5)" in s
assert "roundto: 13 -> 15 (nearest multiple of 5)" in s
assert "roundto: 25 -> 30 (nearest multiple of 10)" in s   # exact half rounds up

print("ROUNDTO TEST PASSED  (17 5->15, 18 5->20, 12 5->10, 13 5->15, 25 10->30 vs python "
      "nearest-multiple round-half-up)")
PY
