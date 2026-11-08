#!/usr/bin/env bash
# Verify the `giccinfo` command reads the GICv2 CPU interface over MMIO (complements
# gicinfo, which reads the distributor). On QEMU virt GICC_IIDR = 0x0002043b:
# implementer 0x43b (ARM), architecture GICv2. CTLR enable=on and PMR=0xff reflect
# the kernel's gic::init (CPU interface enabled, priority mask wide open).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4826,server,nowait \
    -qmp tcp:127.0.0.1:4827,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/giccinfo.out"
python3 scripts/shell-demo.py 4826 4827 "${TMPDIR:-/tmp}/giccinfo.png" "giccinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "giccinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

if "giccinfo: GICC_IIDR = 0x" not in s:
    print("GICCINFO TEST FAILED (missing raw GICC_IIDR)"); sys.exit(1)
# Deterministic on QEMU virt (GICv2, ARM implementer).
if "arch = GICv2" not in s:
    print("GICCINFO TEST FAILED (arch != GICv2)"); sys.exit(1)
if "implementer = 0x43b" not in s:
    print("GICCINFO TEST FAILED (implementer != 0x43b ARM)"); sys.exit(1)
if "giccinfo: CTLR enable = " not in s or "PMR = 0x" not in s:
    print("GICCINFO TEST FAILED (missing CTLR/PMR line)"); sys.exit(1)

print("GICCINFO TEST PASSED  (GICC_IIDR decoded: ARM implementer, GICv2, CTLR+PMR read)")
PY
