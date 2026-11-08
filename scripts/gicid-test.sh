#!/usr/bin/env bash
# Verify the `gicid` command reads + decodes GICD_IIDR (the GICv2 distributor's
# Implementer Identification Register) over MMIO -- distinct from gicinfo
# (GICD_TYPER topology) and giccinfo (GICC_IIDR CPU interface). On QEMU virt the
# distributor's IIDR is deterministic: 0x0000043b => implementer 0x43b (ARM),
# product 0x00, variant 0, revision 0 (observed at offset 0x008 from GIC_DIST_BASE).
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
OUT="${TMPDIR:-/tmp}/gicid.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/gicid.png" "gicid" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "gicid:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "gicid: GICD_IIDR = 0x0000043b",
    "gicid: implementer = 0x43b (ARM)  product = 0x00",
    "gicid: variant = 0  revision = 0",
]
for want in checks:
    if want not in s:
        print(f"GICID TEST FAILED (missing '{want}')"); sys.exit(1)

print("GICID TEST PASSED  (GICD_IIDR=0x0000043b -> implementer 0x43b ARM, product 0x00, variant 0, revision 0)")
PY
