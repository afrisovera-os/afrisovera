#!/usr/bin/env bash
# Verify the `revnum` command reverses a number's decimal digits + flags palindromes:
# 1234 -> 4321 (not a palindrome); 1221 -> 1221 (palindrome); 1200 -> 21 (leading
# zeros drop). The test recomputes reverse + palindrome in python for a self-check.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4956,server,nowait \
    -qmp tcp:127.0.0.1:4957,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/revnum.out"
python3 scripts/shell-demo.py 4956 4957 "${TMPDIR:-/tmp}/revnum.png" \
    "revnum 1234" "revnum 1221" "revnum 1200" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "revnum:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    r = int(str(n)[::-1])
    tag = "palindrome" if str(n) == str(n)[::-1] else "not a palindrome"
    return f"revnum: {n} reversed = {r} ({tag})"

for n in [1234, 1221, 1200]:
    want = line(n)
    if want not in s:
        print(f"REVNUM TEST FAILED (expected '{want}')"); sys.exit(1)

print("REVNUM TEST PASSED  (1234->4321 not-pal, 1221->1221 pal, 1200->21 leading-zero-drop)")
PY
