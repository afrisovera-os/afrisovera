#!/usr/bin/env bash
# Verify the `cacheinfo` command decodes CTR_EL0 (the AArch64 Cache Type Register)
# for the instruction- and data-cache minimum line sizes. On QEMU's -cpu cortex-a72
# CTR_EL0 = 0x8444c004 (IminLine=4, DminLine=4 -> 64-byte I/D lines; CWG=ERG=64),
# so the decode is deterministic. Assertions are also structural (power-of-two,
# multiple of 4, 16..256) so the suite stays honest on any conformant CPU.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4742,server,nowait \
    -qmp tcp:127.0.0.1:4743,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/cacheinfo.out"
python3 scripts/shell-demo.py 4742 4743 "${TMPDIR:-/tmp}/cacheinfo.png" "cacheinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cacheinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "cacheinfo: CTR_EL0 = 0x" not in s:
    print("CACHEINFO TEST FAILED (missing CTR_EL0 raw value)"); sys.exit(1)

def line_size(kind):
    m = re.search(rf"{kind} min line = (\d+) bytes", s)
    if not m:
        print(f"CACHEINFO TEST FAILED (missing {kind} min line)"); sys.exit(1)
    return int(m.group(1))

def check(kind, n):
    if not (16 <= n <= 256):
        print(f"CACHEINFO TEST FAILED ({kind} line {n} out of range 16..256)"); sys.exit(1)
    if n % 4 != 0:
        print(f"CACHEINFO TEST FAILED ({kind} line {n} not a multiple of 4)"); sys.exit(1)
    if n & (n - 1) != 0:
        print(f"CACHEINFO TEST FAILED ({kind} line {n} not a power of two)"); sys.exit(1)

ic = line_size("icache")
dc = line_size("dcache")
check("icache", ic)
check("dcache", dc)

# Deterministic on QEMU cortex-a72: 64-byte I/D lines.
if ic != 64 or dc != 64:
    print(f"CACHEINFO TEST FAILED (expected 64-byte lines on cortex-a72, got i={ic} d={dc})"); sys.exit(1)

print(f"CACHEINFO TEST PASSED  (CTR_EL0 decoded: icache={ic}B, dcache={dc}B -- power-of-two, 16..256)")
PY
