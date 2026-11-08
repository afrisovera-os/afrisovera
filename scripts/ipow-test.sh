#!/usr/bin/env bash
# Verify `ipow <base> <exp>` computes the plain integer power b^e in u128 (non-modular):
# 2 10 -> 1024, 3 5 -> 243, 7 3 -> 343, 10 20 -> 10^20, 2 200 -> overflows u128. Distinct from
# pow (modular). Self-checking: python b**e with the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/ipow.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ipow.png" \
    "echo warmup" "ipow 2 10" "ipow 3 5" "ipow 7 3" \
    "ipow 10 20" "ipow 2 200" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ipow:|usage: ipow" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

U128 = (1 << 128) - 1
def line(b, e):
    v = b ** e                              # DERIVED plain integer power
    if v > U128:
        return f"ipow: {b}^{e} overflows u128"
    return f"ipow: {b}^{e} = {v}"

for b, e in [(2, 10), (3, 5), (7, 3), (10, 20), (2, 200)]:
    w = line(b, e)
    if w not in s:
        print(f"IPOW TEST FAILED (b={b} e={e} missing '{w}')"); sys.exit(1)

# pin the notable cases: power of 2, small, cube, large-but-fits, overflow
assert "ipow: 2^10 = 1024" in s
assert "ipow: 3^5 = 243" in s
assert "ipow: 7^3 = 343" in s
assert "ipow: 10^20 = 100000000000000000000" in s
assert "ipow: 2^200 overflows u128" in s

print("IPOW TEST PASSED  (2^10->1024, 3^5->243, 7^3->343, 10^20->1e20, 2^200->overflow "
      "vs python b**e)")
PY
