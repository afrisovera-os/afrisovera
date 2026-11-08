#!/usr/bin/env bash
# Verify `titlecount <text>` counts TITLE-CASE words -- first char uppercase, no other uppercase.
# "The Quick brown Fox" -> 3 of 4 (brown excluded); "HELLO World" -> 1 of 2 (HELLO all-caps);
# "one two three" -> 0 of 3. Self-checking: python replicates the title-case predicate.
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
OUT="${TMPDIR:-/tmp}/titlecount.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/titlecount.png" \
    "echo warmup" "titlecount The Quick brown Fox" "titlecount HELLO World" \
    "titlecount one two three" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "titlecount:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def title_case_word(w):              # DERIVED reference: first upper, no other upper
    return w[0].isupper() and w[0].isascii() and not any(c.isupper() for c in w[1:] if c.isascii())

def counts(t):
    ws = t.split()
    return sum(1 for w in ws if title_case_word(w)), len(ws)

cases = ["The Quick brown Fox", "HELLO World", "one two three"]
for t in cases:
    tc, tot = counts(t)
    want = f"titlecount: {tc} of {tot} words are title case"
    if want not in s:
        print(f"TITLECOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

assert counts("The Quick brown Fox") == (3, 4)
assert counts("HELLO World") == (1, 2)
assert counts("one two three") == (0, 3)
print("TITLECOUNT TEST PASSED  (3/4, 1/2, 0/3 vs python title-case predicate)")
PY
