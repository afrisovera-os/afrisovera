#!/usr/bin/env bash
# Verify `charfromend <n> <text>` returns the Nth char from the END: 1 hello->o, 2 hello->l,
# 5 hello->h, 6 hello->(none), 2 a,b->a (b at end, comma 2nd from end -> ','? see below).
# End-anchored mirror of nthchar. Self-checking: python text[-n].
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
OUT="${TMPDIR:-/tmp}/charfromend.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/charfromend.png" \
    "echo warmup" "charfromend 1 hello" "charfromend 2 hello" "charfromend 5 hello" \
    "charfromend 6 hello" "charfromend 2 a,b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "charfromend:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    return "charfromend: " + (text[-n] if 1 <= n <= len(text) else "(none)")

cases = [(1, "hello"), (2, "hello"), (5, "hello"), (6, "hello"), (2, "a,b")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"CHARFROMEND TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: last, second-last, first-via-end, out-of-range, punctuation counted
assert "charfromend: o" in s
assert "charfromend: l" in s
assert "charfromend: h" in s          # 5th from end of "hello" = first char
assert "charfromend: (none)" in s     # out of range
assert "charfromend: ," in s          # "a,b" 2nd from end = ',' (punctuation counted)

print("CHARFROMEND TEST PASSED  (1 hello->o, 2 hello->l, 5 hello->h, 6 hello->(none), "
      "2 a,b->',' vs python text[-n])")
PY
