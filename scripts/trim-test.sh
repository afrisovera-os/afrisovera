#!/usr/bin/env bash
# Verify `trim <chars> <text>` strips all leading/trailing chars in the set (Python str.strip):
# "x xxhelloxx" -> hello, "0 004200" -> 42, "/ /a/b/" -> a/b, "ab abcba" -> c (both a,b), "x
# hello" -> hello (no change). Interior untouched. Distinct from strippunct/despace/stripprefix.
# Self-checking: python's text.strip(chars) is the reference.
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
OUT="${TMPDIR:-/tmp}/trim.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/trim.png" \
    "echo warmup" "trim x xxhelloxx" "trim 0 004200" "trim / /a/b/" \
    "trim ab abcba" "trim x hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "trim:|usage: trim" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(chars, text):
    return "trim: " + text.strip(chars)   # DERIVED via python str.strip(set)

cases = [("x", "xxhelloxx"), ("0", "004200"), ("/", "/a/b/"),
         ("ab", "abcba"), ("x", "hello")]
for chars, text in cases:
    w = want(chars, text)
    if w not in s:
        print(f"TRIM TEST FAILED (chars '{chars}' missing '{w}')"); sys.exit(1)

# pin the notable branches: single char both ends, leading/trailing zeros, path slashes,
# multi-char set, no-op passthrough
assert "trim: hello" in s        # covers both "x xxhelloxx"->hello and "x hello"->hello
assert "trim: 42" in s
assert "trim: a/b" in s          # interior slash kept
assert "trim: c" in s            # a and b stripped from both ends

print("TRIM TEST PASSED  (x xxhelloxx->hello, 0 004200->42, / /a/b/->a/b, ab abcba->c, "
      "x hello unchanged vs python str.strip)")
PY
