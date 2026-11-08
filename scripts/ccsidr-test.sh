#!/usr/bin/env bash
# Verify the `ccsidr` command selects the L1 data cache (CSSELR_EL1 = 0) and reads
# its geometry from CCSIDR_EL1 -- sets, ways (associativity), line size, total size.
# Complements cacheinfo (CTR line sizes) and clidr (cache topology). On QEMU virt
# (cortex-a72) this is deterministic: CCSIDR_EL1 = 0x700fe01a => 128 sets, 4 ways,
# 64-byte lines => a 32 KiB L1 data cache (the 64-byte line matches cacheinfo).
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
OUT="${TMPDIR:-/tmp}/ccsidr.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ccsidr.png" "ccsidr" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ccsidr:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "ccsidr: CCSIDR_EL1 (L1D) = 0x700fe01a",
    "ccsidr: sets = 128  ways = 4  line = 64 bytes",
    "ccsidr: L1 data cache size = 32768 bytes (32 KiB)",
]
for want in checks:
    if want not in s:
        print(f"CCSIDR TEST FAILED (missing '{want}')"); sys.exit(1)

# Sanity: sets * ways * line == size.
assert 128 * 4 * 64 == 32768
print("CCSIDR TEST PASSED  (CCSIDR_EL1=0x700fe01a -> 128 sets x 4 ways x 64B = 32 KiB L1D)")
PY
