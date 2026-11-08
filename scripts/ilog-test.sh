#!/usr/bin/env bash
# Verify `ilog <b> <n>` computes floor(log_b(n)): ilog 10 999 = 2, ilog 10 1000 = 3 (exact),
# ilog 2 1024 = 10 (exact), ilog 3 27 = 3 (exact), ilog 7 6 = 0. Generalizes ilog2 to any base.
# Self-checking: python computes floor(log_b n) by repeated INTEGER DIVISION (independent of
# the kernel's repeated-multiplication method).
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
OUT="${TMPDIR:-/tmp}/ilog.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ilog.png" \
    "echo warmup" "ilog 10 999" "ilog 10 1000" "ilog 2 1024" \
    "ilog 3 27" "ilog 7 6" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ilog:|usage: ilog" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(b, n):
    k, x = 0, n
    while x >= b:                     # repeated integer division (INDEPENDENT method)
        x //= b; k += 1
    tag = " (exact power)" if b ** k == n else ""
    return f"ilog: floor(log{b}({n})) = {k}{tag}"

for b, n in [(10, 999), (10, 1000), (2, 1024), (3, 27), (7, 6)]:
    w = line(b, n)
    if w not in s:
        print(f"ILOG TEST FAILED (b={b} n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: floor below a power, exact power, exact power of 2, exact, zero
assert "ilog: floor(log10(999)) = 2" in s
assert "ilog: floor(log10(1000)) = 3 (exact power)" in s
assert "ilog: floor(log2(1024)) = 10 (exact power)" in s
assert "ilog: floor(log3(27)) = 3 (exact power)" in s
assert "ilog: floor(log7(6)) = 0" in s

print("ILOG TEST PASSED  (10/999->2, 10/1000->3, 2/1024->10, 3/27->3, 7/6->0 "
      "vs python repeated-division log)")
PY
