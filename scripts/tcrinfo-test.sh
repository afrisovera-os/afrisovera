#!/usr/bin/env bash
# Verify the `tcrinfo` command reads and decodes TCR_EL1 (Translation Control
# Register). The kernel's paging config is fixed, so the decode is deterministic:
# on this kernel TCR_EL1 = 0x0000000200803519 -> 39-bit TTBR0 VA (T0SZ=25),
# 4KB translation granule, 40-bit physical addressing.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4756,server,nowait \
    -qmp tcp:127.0.0.1:4757,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/tcrinfo.out"
python3 scripts/shell-demo.py 4756 4757 "${TMPDIR:-/tmp}/tcrinfo.png" "tcrinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tcrinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
if not re.search(r"TCR_EL1 = 0x[0-9a-f]{16}", s):
    print("TCRINFO TEST FAILED (no raw TCR_EL1 value printed)"); sys.exit(1)
m = re.search(r"TTBR0 VA size = (\d+) bits", s)
if not m:
    print("TCRINFO TEST FAILED (no VA size printed)"); sys.exit(1)
va = int(m.group(1))
if not (32 <= va <= 52):
    print(f"TCRINFO TEST FAILED (VA size = {va}; expected 32..52)"); sys.exit(1)
gm = re.search(r"TTBR0 granule = (4KB|16KB|64KB)", s)
if not gm:
    print("TCRINFO TEST FAILED (granule did not decode to a real 4KB/16KB/64KB value)"); sys.exit(1)
# Exact, deterministic for this kernel: 39-bit VA + 4KB granule.
if va != 39 or gm.group(1) != "4KB":
    print(f"TCRINFO TEST FAILED (expected 39-bit VA + 4KB granule; got {va}-bit + {gm.group(1)})"); sys.exit(1)
print(f"TCRINFO TEST PASSED  (TTBR0 {va}-bit VA, {gm.group(1)} granule)")
PY
