#!/usr/bin/env bash
# Verify `repeateach <n> <text>` repeats each character n times: "3 abc" -> aaabbbccc, "2 abc"
# -> aabbcc (= double), "1 hello" -> hello, "4 xy" -> xxxxyyyy, "0 abc" -> "" (empty). General
# form of double. Distinct from repeat/spaceout. Self-checking: python "".join(c*n for c).
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
OUT="${TMPDIR:-/tmp}/repeateach.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/repeateach.png" \
    "echo warmup" "repeateach 3 abc" "repeateach 2 abc" "repeateach 1 hello" \
    "repeateach 4 xy" "repeateach 0 abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "repeateach:|usage: repeateach" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):
    return "repeateach: " + "".join(c * n for c in text)   # DERIVED via python

cases = [(3, "abc"), (2, "abc"), (1, "hello"), (4, "xy"), (0, "abc")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"REPEATEACH TEST FAILED (n={n} text={text} missing '{w}')"); sys.exit(1)

# pin the notable branches: triple, double (=double cmd), identity n=1, quadruple, empty n=0
assert "repeateach: aaabbbccc" in s
assert "repeateach: aabbcc" in s
assert "repeateach: hello" in s
assert "repeateach: xxxxyyyy" in s

print("REPEATEACH TEST PASSED  (3 abc->aaabbbccc, 2 abc->aabbcc, 1 hello->hello, 4 xy->xxxxyyyy, "
      "0 abc->empty vs python join(c*n))")
PY
