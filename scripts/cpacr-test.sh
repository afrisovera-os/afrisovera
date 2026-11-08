#!/usr/bin/env bash
# Verify the `cpacr` command reads + decodes CPACR_EL1 (Architectural Feature
# Access Control): the FPEN field (FP/SIMD trap/enable state) and ZEN (SVE). This
# kernel enables FP/SIMD at boot (it runs SHA/SIMD self-tests), so CPACR_EL1 is a
# FIXED boot config = deterministic: 0x0000000000300000 => FPEN=3 (FP/SIMD enabled,
# no trap), ZEN=0 (no SVE). Distinct from mmuinfo (SCTLR).
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
OUT="${TMPDIR:-/tmp}/cpacr.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/cpacr.png" "cpacr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cpacr:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "cpacr: CPACR_EL1 = 0x0000000000300000",
    "cpacr: FPEN = 3 -- FP/SIMD enabled (no trap)",
    "cpacr: ZEN = 0 (SVE access)",
]
for want in checks:
    if want not in s:
        print(f"CPACR TEST FAILED (missing '{want}')"); sys.exit(1)

print("CPACR TEST PASSED  (CPACR_EL1=0x300000 -> FPEN 3 FP/SIMD enabled, ZEN 0 no SVE)")
PY
