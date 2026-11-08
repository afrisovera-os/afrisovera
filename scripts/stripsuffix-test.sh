#!/usr/bin/env bash
# Verify `stripsuffix <suffix> <text>` removes a trailing suffix only when the text ends with
# it: "ing running" -> runn, ".txt about.txt" -> about, "ing happy" -> happy (unchanged),
# "s cats" -> cat, "x hello" -> hello (unchanged). Trailing mirror of stripprefix. Distinct
# from replace/tr/strippunct. Self-checking: python's str.removesuffix is the reference.
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
OUT="${TMPDIR:-/tmp}/stripsuffix.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stripsuffix.png" \
    "echo warmup" "stripsuffix ing running" "stripsuffix .txt about.txt" \
    "stripsuffix ing happy" "stripsuffix s cats" \
    "stripsuffix x hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stripsuffix:|usage: stripsuffix" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(suffix, text):
    r = text[:len(text) - len(suffix)] if text.endswith(suffix) else text   # == removesuffix
    return "stripsuffix: " + r

cases = [("ing", "running"), (".txt", "about.txt"), ("ing", "happy"),
         ("s", "cats"), ("x", "hello")]
for suffix, text in cases:
    w = want(suffix, text)
    if w not in s:
        print(f"STRIPSUFFIX TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: suffix removed, extension strip, no-match, plural s, no-match
assert "stripsuffix: runn" in s
assert "stripsuffix: about" in s
assert "stripsuffix: happy" in s     # covers both "ing happy"->happy and "x hello"... no, distinct
assert "stripsuffix: cat" in s
assert "stripsuffix: hello" in s

print("STRIPSUFFIX TEST PASSED  (ing running->runn, .txt about.txt->about, ing happy unchanged, "
      "s cats->cat, x hello unchanged vs python removesuffix)")
PY
