#!/usr/bin/env bash
# Verify `nthchar <n> <text>` returns the Nth (1-based) CHARACTER of inline text counting EVERY
# byte: "1 hello"->h, "5 hello"->o, "2 a,b"->, (punctuation counted, unlike nthletter->b),
# "6 hello"->(none) out of range, "4 abcd"->d. Inline byte-indexer vs charat(FILE)/nthletter(letters
# only). Self-checking: python text[n-1] if 1<=n<=len else (none).
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
OUT="${TMPDIR:-/tmp}/nthchar.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthchar.png" \
    "echo warmup" "nthchar 1 hello" "nthchar 5 hello" "nthchar 2 a,b" \
    "nthchar 6 hello" "nthchar 4 abcd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthchar:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def nthchar(n, text):                          # DERIVED reference
    return text[n - 1] if 1 <= n <= len(text) else "(none)"

cases = [(1, "hello"), (5, "hello"), (2, "a,b"), (6, "hello"), (4, "abcd")]
for n, text in cases:
    w = f"nthchar: {nthchar(n, text)}"
    if w not in s:
        print(f"NTHCHAR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: normal index, punctuation counted, out-of-range
assert "nthchar: h" in s
assert "nthchar: o" in s
assert "nthchar: ," in s          # punctuation counted (nthletter would give 'b')
assert "nthchar: (none)" in s     # out of range
assert "nthchar: d" in s

print("NTHCHAR TEST PASSED  (1 hello->h, 5 hello->o, 2 a,b->',' , 6 hello->(none), 4 abcd->d "
      "vs python text[n-1]; counts punctuation unlike nthletter)")
PY
