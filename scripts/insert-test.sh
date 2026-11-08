#!/usr/bin/env bash
# Verify `insert <pos> <str> <text>` inserts str at 1-based position pos (pos=1 prepends, past
# the end appends): "3 XY hello" -> heXYllo, "1 >> abc" -> >>abc, "4 - abc" -> abc-, "9 ! ab"
# -> ab! (clamped), "3 __ abcde" -> ab__cde. Constructive complement of substr. Self-checking:
# python text[:idx]+ins+text[idx:] with idx=min(pos-1,len).
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
OUT="${TMPDIR:-/tmp}/insert.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/insert.png" \
    "echo warmup" "insert 3 XY hello" "insert 1 >> abc" "insert 4 - abc" \
    "insert 9 ! ab" "insert 3 __ abcde" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "insert:|usage: insert" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(pos, ins, text):
    idx = min(pos - 1, len(text))                    # DERIVED: clamp to end, then splice
    return "insert: " + text[:idx] + ins + text[idx:]

cases = [(3, "XY", "hello"), (1, ">>", "abc"), (4, "-", "abc"),
         (9, "!", "ab"), (3, "__", "abcde")]
for pos, ins, text in cases:
    w = want(pos, ins, text)
    if w not in s:
        print(f"INSERT TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mid insert, prepend, append at end, clamp-append, mid multi-char
assert "insert: heXYllo" in s
assert "insert: >>abc" in s
assert "insert: abc-" in s
assert "insert: ab!" in s          # pos 9 clamped to append
assert "insert: ab__cde" in s

print("INSERT TEST PASSED  (3 XY hello->heXYllo, 1 >> abc->>>abc, 4 - abc->abc-, 9 ! ab->ab!, "
      "3 __ abcde->ab__cde vs python splice)")
PY
