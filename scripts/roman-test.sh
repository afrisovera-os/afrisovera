#!/usr/bin/env bash
# Verify the `roman` command converts a decimal 1..3999 to standard subtractive
# Roman numerals (deterministic): 4->IV, 1994->MCMXCIV, 3999->MMMCMXCIX, and
# out-of-range inputs (0, 4000) fall through to the usage line.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4748,server,nowait \
    -qmp tcp:127.0.0.1:4749,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/roman.out"
python3 scripts/shell-demo.py 4748 4749 "${TMPDIR:-/tmp}/roman.png" \
    "roman 4" "roman 1994" "roman 3999" "roman 0" "roman 4000" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "roman: |usage: roman" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "roman: 4 = IV",
    "roman: 1994 = MCMXCIV",
    "roman: 3999 = MMMCMXCIX",
    "usage: roman <1..3999>",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"ROMAN TEST FAILED (missing: {missing})"); sys.exit(1)
print("ROMAN TEST PASSED  (4->IV, 1994->MCMXCIV, 3999->MMMCMXCIX; 0/4000 rejected)")
PY
