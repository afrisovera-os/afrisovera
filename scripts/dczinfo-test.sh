#!/usr/bin/env bash
# Verify the `dczinfo` command decodes DCZID_EL0 (Data Cache Zero ID Register). On
# QEMU -cpu cortex-a72 it reads 0x00000004: BS=4 -> 64-byte DC ZVA block, DZP=0
# (DC ZVA permitted). Structural checks keep it honest on any conformant CPU.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4932,server,nowait \
    -qmp tcp:127.0.0.1:4933,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/dczinfo.out"
python3 scripts/shell-demo.py 4932 4933 "${TMPDIR:-/tmp}/dczinfo.png" "dczinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dczinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "dczinfo: DCZID_EL0 = 0x" not in s:
    print("DCZINFO TEST FAILED (missing raw register value)"); sys.exit(1)

m = re.search(r"DC ZVA block size = (\d+) bytes", s)
if not m:
    print("DCZINFO TEST FAILED (missing block-size line)"); sys.exit(1)
n = int(m.group(1))
if not (16 <= n <= 2048 and n % 4 == 0 and (n & (n - 1)) == 0):
    print(f"DCZINFO TEST FAILED (block size {n} not a sane power-of-two)"); sys.exit(1)

if not re.search(r"dczinfo: DC ZVA (permitted|prohibited)", s):
    print("DCZINFO TEST FAILED (missing permitted/prohibited line)"); sys.exit(1)

# Deterministic on QEMU cortex-a72.
if "DC ZVA block size = 64 bytes (BS=4)" not in s or "DC ZVA permitted" not in s:
    print("DCZINFO TEST FAILED (expected 64-byte block, permitted on cortex-a72)"); sys.exit(1)

print("DCZINFO TEST PASSED  (DCZID_EL0 decoded: 64-byte DC ZVA block, permitted)")
PY
