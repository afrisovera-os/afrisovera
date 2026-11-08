#!/usr/bin/env bash
# Verify the `clidr` command reads + decodes CLIDR_EL1 (the Cache Level ID
# Register) -- the cache-hierarchy topology, distinct from cacheinfo (CTR_EL0
# line sizes). On QEMU virt (cortex-a72) CLIDR_EL1 is deterministic: 0x0a200023
# => L1 = I+D (separate I & D caches), L2 = unified, LoC = 2, LoUU = 1, LoUIS = 1.
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
OUT="${TMPDIR:-/tmp}/clidr.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/clidr.png" "clidr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "clidr:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "clidr: CLIDR_EL1 = 0x0a200023",
    "clidr: L1 = I+D",
    "clidr: L2 = unified",
    "clidr: LoC = 2  LoUU = 1  LoUIS = 1",
]
for want in checks:
    if want not in s:
        print(f"CLIDR TEST FAILED (missing '{want}')"); sys.exit(1)

print("CLIDR TEST PASSED  (CLIDR_EL1=0x0a200023 -> L1 I+D, L2 unified, LoC 2, LoUU 1, LoUIS 1)")
PY
