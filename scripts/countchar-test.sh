#!/usr/bin/env bash
# Verify the `countchar <ch> <text>` command counts how many times a single
# character (the first char of <ch>) occurs in the INLINE text argument -- the
# inline-text analog of charcount (which reads a VFS file). Self-checking:
# python's str.count recomputes each expected count.
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
OUT="${TMPDIR:-/tmp}/countchar.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countchar.png" \
    "echo warmup" "countchar l hello world" "countchar o hello world" \
    "countchar z hello world" "countchar a banana" "countchar" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countchar:|usage: countchar" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (char, text) cases -> the count is over the WHOLE inline text (incl. spaces).
cases = [
    ("l", "hello world"),
    ("o", "hello world"),
    ("z", "hello world"),   # absent -> 0
    ("a", "banana"),
]
for ch, t in cases:
    want = f"countchar: {ch} occurs {t.count(ch)} times"
    if want not in s:
        print(f"COUNTCHAR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly
assert "countchar: l occurs 3 times" in s
assert "countchar: o occurs 2 times" in s
assert "countchar: z occurs 0 times" in s   # absent char reports 0
assert "countchar: a occurs 3 times" in s

if "usage: countchar <ch> <text>" not in s:
    print("COUNTCHAR TEST FAILED (missing usage line for no arg)"); sys.exit(1)

print("COUNTCHAR TEST PASSED  (l->3, o->2, z->0, a(banana)->3 vs python str.count; "
      "no-arg -> usage)")
PY
