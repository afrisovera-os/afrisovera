#!/usr/bin/env bash
# Verify `wordat <n> <text>` prints the Nth (1-based) whitespace-separated word of inline text:
# the inline analogue of wordn (which reads a VFS file), just as charat reads one char. wordat 1
# ... -> the, wordat 3 ... -> brown, out-of-range -> "text has fewer than N words". Self-checking:
# python splits the text and indexes the Nth word for the reference.
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
OUT="${TMPDIR:-/tmp}/wordat.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordat.png" \
    "echo warmup" "wordat 1 the quick brown fox" "wordat 3 the quick brown fox" \
    "wordat 4 the quick brown fox" "wordat 9 the quick brown fox" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordat:|usage: wordat" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

TEXT = "the quick brown fox"
words = TEXT.split()

def message(n):  # mirror the kernel: nth (1-based) word, else the shortfall message
    if 1 <= n <= len(words):
        return f"wordat: {words[n-1]}"
    return f"wordat: text has fewer than {n} words"

for n in [1, 3, 4, 9]:
    want = message(n)   # every value DERIVED by python, never hand-typed
    if want not in s:
        print(f"WORDAT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: first/interior/last word + the out-of-range message
assert f"wordat: {words[0]}" in s     # word 1
assert f"wordat: {words[2]}" in s     # word 3 (interior)
assert f"wordat: {words[3]}" in s     # word 4 (last)
assert "wordat: text has fewer than 9 words" in s

print(f"WORDAT TEST PASSED  (1->{words[0]}, 3->{words[2]}, 4->{words[3]}, 9->shortfall "
      "vs python split index)")
PY
