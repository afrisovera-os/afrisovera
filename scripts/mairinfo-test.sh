#!/usr/bin/env bash
# Verify the `mairinfo` command reads and decodes MAIR_EL1 (Memory Attribute
# Indirection Register). The kernel's attribute layout is fixed, so the decode is
# deterministic: on this kernel MAIR_EL1 = 0x000000000000ff00 ->
#   Attr0 = 0x00 (Device-nGnRnE)   -- for MMIO / device memory
#   Attr1 = 0xff (Normal-WriteBack-RW-Alloc)  -- for cacheable RAM
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4778,server,nowait \
    -qmp tcp:127.0.0.1:4779,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/mairinfo.out"
python3 scripts/shell-demo.py 4778 4779 "${TMPDIR:-/tmp}/mairinfo.png" "mairinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mairinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"MAIR_EL1 = 0x([0-9a-f]{16})", s)
if not m or int(m.group(1), 16) == 0:
    print("MAIRINFO TEST FAILED (no non-zero MAIR_EL1 value)"); sys.exit(1)
want = ["Attr0 = 0x00 (Device-nGnRnE)", "Attr1 = 0xff (Normal-WriteBack-RW-Alloc)"]
missing = [w for w in want if w not in s]
if missing:
    print(f"MAIRINFO TEST FAILED (missing decoded attributes: {missing})"); sys.exit(1)
print(f"MAIRINFO TEST PASSED  (MAIR=0x{m.group(1)}; Attr0 Device-nGnRnE, Attr1 Normal-WB)")
PY
