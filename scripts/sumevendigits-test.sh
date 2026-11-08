#!/usr/bin/env bash
# Verify `sumevendigits <n>` sums the EVEN-valued decimal digits: 12345->6 (2+4), 2468->20,
# 13579->0 (all odd), 246->12, 0->0. Distinct from digitsum (all)/evendigits (counts)/sumdigitsquares.
# Self-checking: python sum(int(d) for d in str(n) if int(d)%2==0).
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
OUT="${TMPDIR:-/tmp}/sumevendigits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumevendigits.png" \
    "echo warmup" "sumevendigits 12345" "sumevendigits 2468" "sumevendigits 13579" \
    "sumevendigits 246" "sumevendigits 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumevendigits:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference
    return f"sumevendigits: {n} -> {sum(int(d) for d in str(n) if int(d) % 2 == 0)}"

for n in [12345, 2468, 13579, 246, 0]:
    w = want(n)
    if w not in s:
        print(f"SUMEVENDIGITS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mixed, all even, all odd (0), even set, zero
assert "sumevendigits: 12345 -> 6" in s
assert "sumevendigits: 2468 -> 20" in s
assert "sumevendigits: 13579 -> 0" in s     # all odd -> 0
assert "sumevendigits: 246 -> 12" in s
assert "sumevendigits: 0 -> 0" in s

print("SUMEVENDIGITS TEST PASSED  (12345->6, 2468->20, 13579->0, 246->12, 0->0 vs python sum of even digits)")
PY
