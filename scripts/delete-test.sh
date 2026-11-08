#!/usr/bin/env bash
# Verify `delete <chars> <text>` removes all occurrences of any char in the set (like tr -d):
# "l hello" -> heo, "aeiou hello world" -> hll wrld, "0 a0b0c0" -> abc, "xyz hello" -> hello
# (none present), "lo hello world" -> he wrd. Distinct from trim/strippunct/replace. Self-
# checking: python "".join(c for c in text if c not in set).
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
OUT="${TMPDIR:-/tmp}/delete.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/delete.png" \
    "echo warmup" "delete l hello" "delete aeiou hello world" "delete 0 a0b0c0" \
    "delete xyz hello" "delete lo hello world" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "delete:|usage: delete" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(chars, text):
    st = set(chars)
    return "delete: " + "".join(c for c in text if c not in st)   # DERIVED via python

cases = [("l", "hello"), ("aeiou", "hello world"), ("0", "a0b0c0"),
         ("xyz", "hello"), ("lo", "hello world")]
for chars, text in cases:
    w = want(chars, text)
    if w not in s:
        print(f"DELETE TEST FAILED (chars '{chars}' missing '{w}')"); sys.exit(1)

# pin the notable branches: single char, vowel set (disemvowel), digit, no-match, multi-char
assert "delete: heo" in s
assert "delete: hll wrld" in s     # = disemvowel
assert "delete: abc" in s
assert "delete: hello" in s        # xyz none present -> unchanged
assert "delete: he wrd" in s       # remove l and o

print("DELETE TEST PASSED  (l hello->heo, aeiou hello world->hll wrld, 0 a0b0c0->abc, "
      "xyz hello unchanged, lo hello world->he wrd vs python filter)")
PY
