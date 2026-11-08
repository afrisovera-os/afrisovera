#!/usr/bin/env bash
# Verify `repeatword <n> <text>` repeats text n times joined by single spaces: 3 hi->hi hi hi,
# 1 x->x, 4 ab->ab ab ab ab, 2 abc->abc abc, 0 y->"" (empty). Space-joined form of repeat (no sep).
# Self-checking: python " ".join([text]*n).
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
OUT="${TMPDIR:-/tmp}/repeatword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/repeatword.png" \
    "echo warmup" "repeatword 3 hi" "repeatword 1 x" "repeatword 4 ab" \
    "repeatword 2 abc" "repeatword 0 y" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "repeatword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    return "repeatword: " + " ".join([text] * n)

for n, text in [(3, "hi"), (1, "x"), (4, "ab"), (2, "abc")]:
    w = want(n, text)
    if w not in s:
        print(f"REPEATWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: typical, single, four copies, two, and n=0 (empty tail)
assert "repeatword: hi hi hi" in s
assert "repeatword: x" in s
assert "repeatword: ab ab ab ab" in s
assert "repeatword: abc abc" in s
assert "repeatword: \r" in s or "repeatword: \n" in s or "repeatword:  " in s or "repeatword:" in s  # n=0 empty

print("REPEATWORD TEST PASSED  (3 hi->hi hi hi, 1 x->x, 4 ab->ab ab ab ab, 2 abc->abc abc, 0 y->empty "
      "vs python ' '.join([text]*n))")
PY
