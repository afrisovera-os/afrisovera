#!/usr/bin/env bash
# Verify the `mmuinfo` command reads and decodes SCTLR_EL1 (the EL1 System
# Control Register). The MMU is enabled at boot (the boot log says "MMU
# enabled"), so the M bit is definitively 1 -> "MMU on" is deterministic. The
# dcache/icache states are printed but not asserted (config-dependent).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4732,server,nowait \
    -qmp tcp:127.0.0.1:4733,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/mmuinfo.out"
python3 scripts/shell-demo.py 4732 4733 "${TMPDIR:-/tmp}/mmuinfo.png" "mmuinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mmuinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"SCTLR_EL1 = 0x([0-9a-f]{16})", s)
if not m or int(m.group(1), 16) == 0:
    print("MMUINFO TEST FAILED (no non-zero SCTLR_EL1 value printed)"); sys.exit(1)
if "MMU on" not in s:
    print("MMUINFO TEST FAILED (MMU not reported as on -- but the kernel runs with the MMU enabled)"); sys.exit(1)
print(f"MMUINFO TEST PASSED  (SCTLR_EL1=0x{m.group(1)}; MMU on)")
PY
