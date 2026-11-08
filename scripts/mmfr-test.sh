#!/usr/bin/env bash
# Verify the `mmfr` command decodes ID_AA64MMFR0_EL1 (Memory Model Feature Register
# 0). On QEMU -cpu cortex-a72 it reads 0x0000000000001124, so the decode is
# deterministic: PA range 44 bits (code 4), ASID 16 bits, granules 4K=yes 16K=no 64K=yes.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4814,server,nowait \
    -qmp tcp:127.0.0.1:4815,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/mmfr.out"
python3 scripts/shell-demo.py 4814 4815 "${TMPDIR:-/tmp}/mmfr.png" "mmfr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mmfr: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "mmfr: ID_AA64MMFR0_EL1 = 0x" not in s:
    print("MMFR TEST FAILED (missing raw register value)"); sys.exit(1)

m = re.search(r"mmfr: PA range = (\d+) bits", s)
if not m:
    print("MMFR TEST FAILED (missing/garbled PA range line)"); sys.exit(1)
pa = int(m.group(1))
if not (32 <= pa <= 52):
    print(f"MMFR TEST FAILED (PA range {pa} out of 32..52)"); sys.exit(1)

if "mmfr: ASID = " not in s or "mmfr: granules:" not in s:
    print("MMFR TEST FAILED (missing ASID or granules line)"); sys.exit(1)

# Deterministic on QEMU cortex-a72.
for w in ["PA range = 44 bits", "ASID = 16 bits", "granules: 4K=yes 16K=no 64K=yes"]:
    if w not in s:
        print(f"MMFR TEST FAILED (expected '{w}' on cortex-a72)"); sys.exit(1)

print("MMFR TEST PASSED  (ID_AA64MMFR0_EL1 decoded: 44-bit PA, 16-bit ASID, 4K/64K granules)")
PY
