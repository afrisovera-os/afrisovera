#!/usr/bin/env bash
# Verify `deletechar <k> <text>` removes the kth (1-based) char, closing the gap: 3 hello->helo,
# 1 abc->bc, 3 abc->ab, 5 abc->abc (k past end unchanged), 2 hi->h. Position-based single-char
# removal vs delete (SET removal). Self-checking: python text[:k-1]+text[k:] if 1<=k<=len else text.
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
OUT="${TMPDIR:-/tmp}/deletechar.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/deletechar.png" \
    "echo warmup" "deletechar 3 hello" "deletechar 1 abc" "deletechar 3 abc" \
    "deletechar 5 abc" "deletechar 2 hi" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "deletechar:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(k, text):                    # DERIVED reference
    out = text[:k - 1] + text[k:] if 1 <= k <= len(text) else text
    return f"deletechar: {out}"

for k, text in [(3, "hello"), (1, "abc"), (3, "abc"), (5, "abc"), (2, "hi")]:
    w = want(k, text)
    if w not in s:
        print(f"DELETECHAR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: middle, first, last, out-of-range (unchanged), two-char
assert "deletechar: helo" in s
assert "deletechar: bc" in s
assert "deletechar: ab" in s
assert "deletechar: abc" in s      # k past end -> unchanged
assert "deletechar: h" in s

print("DELETECHAR TEST PASSED  (3 hello->helo, 1 abc->bc, 3 abc->ab, 5 abc->abc, 2 hi->h "
      "vs python text[:k-1]+text[k:])")
PY
