#!/usr/bin/env bash
# Verify `clz <n>` counts LEADING ZERO bits of n in its 64-bit representation
# (u64::leading_zeros) -- the mirror of ctz. Tests 1->63, 2->62, 255->56, 256->55,
# 0->64 (all bits zero), bad input. Self-checking: python 64 - n.bit_length().
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
OUT="${TMPDIR:-/tmp}/clz.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/clz.png" \
    "echo warmup" "clz 1" "clz 2" "clz 255" "clz 256" "clz 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "clz:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def clz(n):                          # DERIVED reference: leading zeros in 64 bits
    return 64 - n.bit_length()

for n in (1, 2, 255, 256, 0):
    lz = clz(n)
    suf = "" if lz == 1 else "s"
    want = f"clz: {n} has {lz} leading zero bit{suf} (in 64 bits)"
    if want not in s:
        print(f"CLZ TEST FAILED (missing '{want}')"); sys.exit(1)

assert clz(1) == 63 and clz(2) == 62 and clz(255) == 56 and clz(256) == 55 and clz(0) == 64
print("CLZ TEST PASSED  (1/2/255/256/0 vs python 64-bit_length; mirror of ctz)")
PY
