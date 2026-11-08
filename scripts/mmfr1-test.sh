#!/usr/bin/env bash
# Verify the `mmfr1` command reads + decodes ID_AA64MMFR1_EL1 (Memory Model
# Feature Register 1) -- HAFDBS/VMIDBits/VH/PAN/HPDS/LO/SpecSEI/XNX -- complementing
# `mmfr` (MMFR0: PA range/ASID/granules). On QEMU virt (cortex-a72, ARMv8.0)
# ID_AA64MMFR1_EL1 is deterministic: 0x0 -- none of the v8.1+ memory-model
# features are implemented, so every field is 0 and VMIDBits=0 decodes to 8-bit
# VMID. This proves the read + field decode + VMID derivation.
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
OUT="${TMPDIR:-/tmp}/mmfr1.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mmfr1.png" "mmfr1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mmfr1:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "mmfr1: ID_AA64MMFR1_EL1 = 0x0000000000000000",
    "mmfr1: HAFDBS=0 VMIDBits=0 VH=0 PAN=0",
    "mmfr1: HPDS=0 LO=0 SpecSEI=0 XNX=0",
    "mmfr1: VMID = 8-bit",
]
for want in checks:
    if want not in s:
        print(f"MMFR1 TEST FAILED (missing '{want}')"); sys.exit(1)

print("MMFR1 TEST PASSED  (ID_AA64MMFR1_EL1=0x0 on cortex-a72 -> all fields 0, VMID 8-bit)")
PY
