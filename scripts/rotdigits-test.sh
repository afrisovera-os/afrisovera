#!/usr/bin/env bash
# Verify `rotdigits <n>` rotates the DECIMAL digits LEFT by one: 1234->2341, 90210->2109,
# 1000->1, 5->5, 0->0. Distinct from revnum (reverse) / rotl/rotr (bit rotate). Self-checking:
# python rotate-left-by-one on the digit string, parsed back to int.
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
OUT="${TMPDIR:-/tmp}/rotdigits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotdigits.png" \
    "echo warmup" "rotdigits 1234" "rotdigits 90210" "rotdigits 1000" \
    "rotdigits 5" "rotdigits 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotdigits:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference: rotate digit string left by 1
    d = str(n)
    rot = int(d[1:] + d[0]) if len(d) > 1 else n
    return f"rotdigits: {n} -> {rot}"

cases = [1234, 90210, 1000, 5, 0]
for n in cases:
    w = want(n)
    if w not in s:
        print(f"ROTDIGITS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: plain, leading-zero collapse, trailing zeros, single, zero
assert "rotdigits: 1234 -> 2341" in s
assert "rotdigits: 90210 -> 2109" in s   # leading zero collapses
assert "rotdigits: 1000 -> 1" in s       # 0001
assert "rotdigits: 5 -> 5" in s
assert "rotdigits: 0 -> 0" in s

print("ROTDIGITS TEST PASSED  (1234->2341, 90210->2109, 1000->1, 5->5, 0->0 vs python "
      "rotate-left-by-one)")
PY
