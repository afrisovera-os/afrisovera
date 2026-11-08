#!/usr/bin/env bash
# Verify the `pmcr` command reads + decodes PMCR_EL0 (Performance Monitors Control
# Register): the number of event counters (N), implementer/IDCODE, and cycle-counter
# width -- distinct from `dfr` (which reports the PMU architecture VERSION). On QEMU
# virt (cortex-a72) PMCR_EL0 is deterministic: 0x41023000 => N=6 event counters,
# implementer 0x41 (ARM), IDCODE 0x02, 32-bit cycle counter, all-enable off.
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
OUT="${TMPDIR:-/tmp}/pmcr.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pmcr.png" "pmcr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pmcr:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "pmcr: PMCR_EL0 = 0x41023000",
    "pmcr: event counters (N) = 6",
    "pmcr: implementer = 0x41  IDCODE = 0x02",
    "pmcr: cycle counter = 32-bit  all-enable = off",
]
for want in checks:
    if want not in s:
        print(f"PMCR TEST FAILED (missing '{want}')"); sys.exit(1)

print("PMCR TEST PASSED  (PMCR_EL0=0x41023000 -> N=6 counters, impl 0x41 ARM, IDCODE 0x02, 32-bit cycle ctr)")
PY
