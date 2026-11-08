#!/usr/bin/env bash
# Verify the `ttbr` command reads + decodes TTBR0_EL1/TTBR1_EL1 (the Translation
# Table Base Registers) -- the base addresses of the low (user) and high (kernel)
# page tables + ASID. Complements mmuinfo (SCTLR) and tcrinfo (TCR). On QEMU virt
# this kernel uses a single TTBR0 regime: TTBR0 base is a NON-ZERO RAM address
# (0x4xxxxxxx, deterministic per boot but it drifts as the kernel image grows, so
# we assert its STRUCTURE not the exact value), TTBR1 = 0 (unused), ASID = 0.
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
OUT="${TMPDIR:-/tmp}/ttbr.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ttbr.png" "ttbr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ttbr:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

# Structural: TTBR0 is a non-zero page-table base in RAM (0x4xxxxxxx). The exact
# value drifts as the kernel image grows, so match the range, not the literal.
if not re.search(r"ttbr: TTBR0 base = 0x4[0-9a-f]+  ASID = 0", s):
    print("TTBR TEST FAILED (TTBR0 base not a non-zero 0x4... RAM address with ASID 0)"); sys.exit(1)
if "ttbr: TTBR0_EL1 = 0x" not in s:
    print("TTBR TEST FAILED (missing raw TTBR0_EL1 line)"); sys.exit(1)

# Deterministic facts: this kernel uses a single TTBR0 regime, so TTBR1 is unused.
for want in ("ttbr: TTBR1_EL1 = 0x0000000000000000", "ttbr: TTBR1 base = 0x0"):
    if want not in s:
        print(f"TTBR TEST FAILED (missing '{want}')"); sys.exit(1)

print("TTBR TEST PASSED  (TTBR0 base = non-zero RAM addr, ASID 0; TTBR1 unused (0) -- single-regime MMU)")
PY
