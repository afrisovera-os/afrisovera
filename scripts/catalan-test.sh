#!/usr/bin/env bash
# Verify the `catalan` command computes the nth Catalan number via the exact
# incremental u128 recurrence (deterministic): C(5)=42, C(10)=16796, C(15)=9694845,
# C(20)=6564120420. C(65) is the last that fits u128; C(66)+ report overflow (guarded,
# not silently wrapped).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4848,server,nowait \
    -qmp tcp:127.0.0.1:4849,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/catalan.out"
python3 scripts/shell-demo.py 4848 4849 "${TMPDIR:-/tmp}/catalan.png" \
    "catalan 5" "catalan 10" "catalan 15" "catalan 20" "catalan 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "catalan:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "catalan: C(5) = 42",
    "catalan: C(10) = 16796",
    "catalan: C(15) = 9694845",
    "catalan: C(20) = 6564120420",
    "catalan: C(100) overflows u128",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"CATALAN TEST FAILED (missing: {missing})"); sys.exit(1)
print("CATALAN TEST PASSED  (C(5)=42, C(10)=16796, C(15)=9694845, C(20)=6564120420; C(100) overflow-guarded)")
PY
