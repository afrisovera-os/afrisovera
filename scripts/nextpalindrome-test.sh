#!/usr/bin/env bash
# Verify `nextpalindrome <n>` gives the smallest numeric palindrome > n: 123 -> 131, 99 ->
# 101, 5 -> 6, 191 -> 202, 9 -> 11. Distinct from palindrome (string test) and revnum. Self-
# checking: python scans n+1 upward for str(c)==str(c)[::-1].
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
OUT="${TMPDIR:-/tmp}/nextpalindrome.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextpalindrome.png" \
    "echo warmup" "nextpalindrome 123" "nextpalindrome 99" "nextpalindrome 5" \
    "nextpalindrome 191" "nextpalindrome 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextpalindrome:|usage: nextpalindrome" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def nextpal(n):
    c = n + 1
    while str(c) != str(c)[::-1]:       # DERIVED: scan up for a digit palindrome
        c += 1
    return c

for n in [123, 99, 5, 191, 9]:
    want = f"nextpalindrome: next palindrome after {n} is {nextpal(n)}"    # DERIVED
    if want not in s:
        print(f"NEXTPALINDROME TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "nextpalindrome: next palindrome after 123 is 131" in s
assert "nextpalindrome: next palindrome after 99 is 101" in s
assert "nextpalindrome: next palindrome after 5 is 6" in s
assert "nextpalindrome: next palindrome after 191 is 202" in s
assert "nextpalindrome: next palindrome after 9 is 11" in s

print("NEXTPALINDROME TEST PASSED  (123->131, 99->101, 5->6, 191->202, 9->11 "
      "vs python palindrome scan)")
PY
