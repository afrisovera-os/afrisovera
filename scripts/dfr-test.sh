#!/usr/bin/env bash
# Verify the `dfr` command decodes ID_AA64DFR0_EL1 (Debug Feature Register 0). On
# QEMU -cpu cortex-a72 it reads 0x0000000010305106, so the decode is deterministic:
# DebugVer=6 (ARMv8), 6 hardware breakpoints, 4 watchpoints, PMUv3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4894,server,nowait \
    -qmp tcp:127.0.0.1:4895,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/dfr.out"
python3 scripts/shell-demo.py 4894 4895 "${TMPDIR:-/tmp}/dfr.png" "dfr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dfr: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

if "dfr: ID_AA64DFR0_EL1 = 0x" not in s:
    print("DFR TEST FAILED (missing raw register value)"); sys.exit(1)

# Deterministic on QEMU cortex-a72.
want = [
    "dfr: debug = ARMv8 (DebugVer=6)",
    "dfr: breakpoints = 6  watchpoints = 4",
    "dfr: PMU = PMUv3",
]
for w in want:
    if w not in s:
        print(f"DFR TEST FAILED (expected '{w}')"); sys.exit(1)

print("DFR TEST PASSED  (ID_AA64DFR0_EL1: ARMv8 debug, 6 breakpoints, 4 watchpoints, PMUv3)")
PY
