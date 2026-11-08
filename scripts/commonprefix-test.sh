#!/usr/bin/env bash
# Verify `commonprefix <a> <b>` finds the longest common prefix of two strings: "hello help"
# -> hel, "foobar foo" -> foo, "abc xyz" -> "" (empty), "same same" -> same, "prefix pre" ->
# pre. Distinct from stripprefix/before. Self-checking: python compares byte by byte from the
# start (== os.path.commonprefix for two strings).
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
OUT="${TMPDIR:-/tmp}/commonprefix.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/commonprefix.png" \
    "echo warmup" "commonprefix hello help" "commonprefix foobar foo" "commonprefix abc xyz" \
    "commonprefix same same" "commonprefix prefix pre" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "commonprefix:|usage: commonprefix" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def cp(a, b):
    i = 0
    while i < len(a) and i < len(b) and a[i] == b[i]:   # DERIVED byte-wise common prefix
        i += 1
    return a[:i]

cases = [("hello", "help"), ("foobar", "foo"), ("abc", "xyz"),
         ("same", "same"), ("prefix", "pre")]
for a, b in cases:
    want = "commonprefix: " + cp(a, b)
    if want not in s:
        print(f"COMMONPREFIX TEST FAILED (a={a} b={b} missing '{want}')"); sys.exit(1)

# pin the notable branches: partial overlap, one is prefix of other, no overlap, identical, prefix
assert "commonprefix: hel" in s
assert "commonprefix: foo" in s        # covers both "foobar foo"->foo and "prefix pre"->pre? no, pre distinct
assert "commonprefix: same" in s
assert "commonprefix: pre" in s

print("COMMONPREFIX TEST PASSED  (hello help->hel, foobar foo->foo, abc xyz->empty, "
      "same same->same, prefix pre->pre vs python byte-wise LCP)")
PY
