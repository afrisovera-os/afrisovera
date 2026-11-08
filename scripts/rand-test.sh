#!/usr/bin/env bash
# Sanity-check the hardware-seeded PRNG: `rand 8` must produce 8 distinct-looking
# 32-bit hex values (not stuck/degenerate), and two separate `rand` invocations
# must differ (the generator advances). An RNG seeded from hardware entropy is
# non-deterministic, so this is a statistical sanity test, not an exact match.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4580,server,nowait \
    -qmp tcp:127.0.0.1:4581,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4580 4581 "${TMPDIR:-/tmp}/rand-test.png" "rand 8" "rand 8" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

echo "$SER" | grep -iE "\[rand\]" | sed 's/^/  /'

python3 - <<PY
import sys, re
ser = """$SER"""
lines = [l for l in ser.splitlines() if l.strip().startswith("[rand]")]
if len(lines) < 2:
    print("RAND TEST FAILED (did not capture two rand outputs)"); sys.exit(1)
vals1 = re.findall(r"[0-9a-f]{8}", lines[0])
vals2 = re.findall(r"[0-9a-f]{8}", lines[1])
if len(vals1) != 8:
    print(f"RAND TEST FAILED (expected 8 values, got {len(vals1)})"); sys.exit(1)
if len(set(vals1)) == 1:
    print("RAND TEST FAILED (all 8 values identical -- generator is stuck)"); sys.exit(1)
if vals1 == vals2:
    print("RAND TEST FAILED (two invocations produced identical output -- not advancing)"); sys.exit(1)
distinct = len(set(vals1))
print(f"  first draw: {distinct}/8 distinct values; second draw differs -> generator advances")
print("RAND TEST PASSED  (hardware-seeded PRNG produces varied, non-degenerate output)")
PY
