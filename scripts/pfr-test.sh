#!/usr/bin/env bash
# Verify the `pfr` command decodes ID_AA64PFR0_EL1 (Processor Feature Register 0).
# On QEMU -cpu cortex-a72 (virt, no EL2/EL3) it reads 0x0000000000000022, so the
# decode is deterministic: EL0/EL1 = AArch64+32, EL2/EL3 not implemented, FP + AdvSIMD
# present, no GIC system-register interface (a72 uses memory-mapped GICv2).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4876,server,nowait \
    -qmp tcp:127.0.0.1:4877,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/pfr.out"
python3 scripts/shell-demo.py 4876 4877 "${TMPDIR:-/tmp}/pfr.png" "pfr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pfr: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

if "pfr: ID_AA64PFR0_EL1 = 0x" not in s:
    print("PFR TEST FAILED (missing raw register value)"); sys.exit(1)

# Deterministic on QEMU cortex-a72.
want = [
    "pfr: EL0=AArch64+32 EL1=AArch64+32 EL2=not-impl EL3=not-impl",
    "pfr: FP=yes AdvSIMD=yes GICsysreg=no",
]
for w in want:
    if w not in s:
        print(f"PFR TEST FAILED (expected '{w}')"); sys.exit(1)

print("PFR TEST PASSED  (ID_AA64PFR0_EL1: EL0/EL1 AArch64+32, EL2/EL3 not-impl, FP+SIMD, no GIC sysreg)")
PY
