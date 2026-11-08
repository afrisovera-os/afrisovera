#!/usr/bin/env bash
# Verify the `digitcount <n>` command reports the number of DECIMAL DIGITS of n
# (0 has 1 digit). e.g. 42 -> 2, 12345 -> 5, u64 max -> 20. DISTINCT from digitsum
# (SUM of digits), digitprod (PRODUCT), ilog2 (floor log2), popcount (bit count).
# Self-checking: python's len(str(n)) is the reference.
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
OUT="${TMPDIR:-/tmp}/digitcount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/digitcount.png" \
    "echo warmup" "digitcount 0" "digitcount 7" "digitcount 42" "digitcount 100" \
    "digitcount 999" "digitcount 1000" "digitcount 12345" \
    "digitcount 18446744073709551615" "digitcount" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "digitcount:|usage: digitcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

for n in [0, 7, 42, 100, 999, 1000, 12345, 18446744073709551615]:
    want = f"digitcount: {n} has {len(str(n))} digits"
    if want not in s:
        print(f"DIGITCOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin edge (0 -> 1) and the u64-max 20-digit case
assert len(str(0)) == 1
assert len(str(18446744073709551615)) == 20

if "usage: digitcount <n>" not in s:
    print("DIGITCOUNT TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("DIGITCOUNT TEST PASSED  (0->1, 42->2, 12345->5, u64max->20 exact vs python; "
      "bad->usage)")
PY
