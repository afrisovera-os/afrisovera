#!/usr/bin/env bash
# Verify the `isbinpalindrome <n>` command classifies binary palindromes: n's binary
# representation (no leading zeros) reads the same forwards and backwards (5=101,
# 9=1001, 585=1001001001 IS; 6=110, 10=1010 NOT). Distinct from revnum (decimal) and
# bitrev (all 32 bits). Self-checking: python compares bin(n)[2:] to its reverse.
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
OUT="${TMPDIR:-/tmp}/isbinpalindrome.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 5 (101 IS), 585 (1001001001 IS), 6 (110 NOT), 10 (1010 NOT).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isbinpalindrome.png" \
    "echo warmup" "isbinpalindrome 5" "isbinpalindrome 585" "isbinpalindrome 6" "isbinpalindrome 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isbinpalindrome:|usage: isbinpalindrome" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):
    b = bin(n)[2:]                 # binary without 0b prefix (bin(0) -> '0')
    pal = (b == b[::-1])
    verd = "IS" if pal else "is NOT"
    return f"isbinpalindrome: {n} ({b}) {verd} a binary palindrome"

for n in [5, 585, 6, 10]:
    want = message(n)              # binary string DERIVED by python, never hand-typed
    if want not in s:
        print(f"ISBINPALINDROME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "isbinpalindrome: 5 (101) IS a binary palindrome" in s
assert "isbinpalindrome: 585 (1001001001) IS a binary palindrome" in s
assert "isbinpalindrome: 6 (110) is NOT a binary palindrome" in s
assert "isbinpalindrome: 10 (1010) is NOT a binary palindrome" in s

print("ISBINPALINDROME TEST PASSED  (5->101 IS, 585->1001001001 IS, 6->110 NOT, 10->1010 NOT "
      "vs python; binary-representation palindrome)")
PY
