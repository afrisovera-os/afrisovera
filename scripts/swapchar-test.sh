#!/usr/bin/env bash
# Verify `swapchar <i> <j> <text>` swaps the i-th and j-th (1-based) chars: 1 5 hello->oellh,
# 2 4 abcde->adcbe, 1 1 abc->abc (same idx), 3 9 abc->abc (out of range unchanged), 1 2 ab->ba.
# Char-level analogue of swapwords. Self-checking: python list swap.
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
OUT="${TMPDIR:-/tmp}/swapchar.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swapchar.png" \
    "echo warmup" "swapchar 1 5 hello" "swapchar 2 4 abcde" "swapchar 1 1 abc" \
    "swapchar 3 9 abc" "swapchar 1 2 ab" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapchar:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(i, j, text):                 # DERIVED reference
    b = list(text)
    if 1 <= i <= len(b) and 1 <= j <= len(b):
        b[i - 1], b[j - 1] = b[j - 1], b[i - 1]
    return "swapchar: " + "".join(b)

cases = [(1, 5, "hello"), (2, 4, "abcde"), (1, 1, "abc"), (3, 9, "abc"), (1, 2, "ab")]
for i, j, text in cases:
    w = want(i, j, text)
    if w not in s:
        print(f"SWAPCHAR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: ends swap, interior swap, same idx (no-op), out-of-range, two-char
assert "swapchar: oellh" in s
assert "swapchar: adcbe" in s
assert "swapchar: abc" in s       # same-idx AND out-of-range both -> abc
assert "swapchar: ba" in s

print("SWAPCHAR TEST PASSED  (1 5 hello->oellh, 2 4 abcde->adcbe, 1 1 abc->abc, 3 9 abc->abc, "
      "1 2 ab->ba vs python list swap)")
PY
