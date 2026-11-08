#!/usr/bin/env bash
# Verify the `cpuinfo` command decodes the AArch64 ID system registers. On QEMU's
# `-cpu cortex-a72`, MIDR_EL1 = 0x410fd083, so the decode is deterministic:
# implementer 0x41 (ARM), part 0xd08 (Cortex-A72), running at EL1.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4700,server,nowait \
    -qmp tcp:127.0.0.1:4701,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/cpuinfo.out"
python3 scripts/shell-demo.py 4700 4701 "${TMPDIR:-/tmp}/cpuinfo.png" "cpuinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cpuinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["implementer 0x41 (ARM)", "part 0xd08 (Cortex-A72)", "EL 1"]
missing = [w for w in want if w not in s]
if missing:
    print(f"CPUINFO TEST FAILED (missing substring(s): {missing})"); sys.exit(1)
print("CPUINFO TEST PASSED  (ARM Cortex-A72, EL1 -- decoded from MIDR/MPIDR/CurrentEL)")
PY
