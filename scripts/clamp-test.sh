#!/usr/bin/env bash
# Verify `clamp <n> <lo> <hi>` constrains n to [lo,hi] = max(lo,min(n,hi)): 5 1 10->5 (in range),
# 15 1 10->10 (above->hi), 0 1 10->1 (below->lo), 7 7 7->7, 100 50 60->60. Range-limiting primitive
# vs minmax/median. Self-checking: python max(lo,min(n,hi)).
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
OUT="${TMPDIR:-/tmp}/clamp.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/clamp.png" \
    "echo warmup" "clamp 5 1 10" "clamp 15 1 10" "clamp 0 1 10" \
    "clamp 7 7 7" "clamp 100 50 60" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "clamp:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, lo, hi):                  # DERIVED reference
    return f"clamp: {n} in [{lo}, {hi}] = {max(lo, min(n, hi))}"

cases = [(5, 1, 10), (15, 1, 10), (0, 1, 10), (7, 7, 7), (100, 50, 60)]
for n, lo, hi in cases:
    w = want(n, lo, hi)
    if w not in s:
        print(f"CLAMP TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: in-range, above->hi, below->lo, degenerate, above larger interval
assert "clamp: 5 in [1, 10] = 5" in s
assert "clamp: 15 in [1, 10] = 10" in s     # clamped up to hi
assert "clamp: 0 in [1, 10] = 1" in s       # clamped down to lo
assert "clamp: 7 in [7, 7] = 7" in s
assert "clamp: 100 in [50, 60] = 60" in s

print("CLAMP TEST PASSED  (5 1 10->5, 15 1 10->10, 0 1 10->1, 7 7 7->7, 100 50 60->60 "
      "vs python max(lo,min(n,hi)))")
PY
