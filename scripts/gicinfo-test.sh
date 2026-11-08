#!/usr/bin/env bash
# Verify the `gicinfo` command reads the GICv2 distributor's GICD_TYPER register
# over MMIO and reports the interrupt topology. On QEMU virt this observed
# GICD_TYPER = 0x00000008 -> 288 IRQ lines, distributor enabled. Assertions are
# structural (IRQ lines a positive multiple of 32; distributor enabled) so they
# stay robust across QEMU versions rather than pinning an exact line count.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4738,server,nowait \
    -qmp tcp:127.0.0.1:4739,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/gicinfo.out"
python3 scripts/shell-demo.py 4738 4739 "${TMPDIR:-/tmp}/gicinfo.png" "gicinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "gicinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
if not re.search(r"GICD_TYPER = 0x[0-9a-f]{8}", s):
    print("GICINFO TEST FAILED (no GICD_TYPER value printed)"); sys.exit(1)
m = re.search(r"IRQ lines = (\d+)", s)
if not m:
    print("GICINFO TEST FAILED (no IRQ line count printed)"); sys.exit(1)
n = int(m.group(1))
if n < 32 or n % 32 != 0:
    print(f"GICINFO TEST FAILED (IRQ lines = {n}; expected a positive multiple of 32)"); sys.exit(1)
if "distributor enabled" not in s:
    print("GICINFO TEST FAILED (distributor not reported enabled -- the kernel enables the GIC at boot)"); sys.exit(1)
print(f"GICINFO TEST PASSED  ({n} IRQ lines (multiple of 32); distributor enabled)")
PY
