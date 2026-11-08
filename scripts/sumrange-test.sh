#!/usr/bin/env bash
# Verify `sumrange <a> <b>` sums integers a..b inclusive (Gauss): 1 10->55, 10 20->165, 5 5->5,
# 1 100->5050, 7 3->0 (empty range a>b). General-interval companion of sumto (fixed 1..n).
# Self-checking: python sum(range(a,b+1)) if a<=b else 0.
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
OUT="${TMPDIR:-/tmp}/sumrange.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumrange.png" \
    "echo warmup" "sumrange 1 10" "sumrange 10 20" "sumrange 5 5" \
    "sumrange 1 100" "sumrange 7 3" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumrange:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b):                       # DERIVED reference
    total = sum(range(a, b + 1)) if a <= b else 0
    return f"sumrange: {a}+...+{b} = {total}"

for a, b in [(1, 10), (10, 20), (5, 5), (1, 100), (7, 3)]:
    w = want(a, b)
    if w not in s:
        print(f"SUMRANGE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: base, offset interval, single, larger, empty (a>b)
assert "sumrange: 1+...+10 = 55" in s
assert "sumrange: 10+...+20 = 165" in s
assert "sumrange: 5+...+5 = 5" in s
assert "sumrange: 1+...+100 = 5050" in s
assert "sumrange: 7+...+3 = 0" in s      # empty range

print("SUMRANGE TEST PASSED  (1 10->55, 10 20->165, 5 5->5, 1 100->5050, 7 3->0 vs python "
      "sum(range(a,b+1)))")
PY
