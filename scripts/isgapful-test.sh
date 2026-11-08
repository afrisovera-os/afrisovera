#!/usr/bin/env bash
# Verify the `isgapful <n>` command classifies gapful numbers: n (>=100) divisible
# by the two-digit number formed from its first and last decimal digits (100->10
# IS, 108->18 IS, 106->16 NOT). n<100 is reported out of range. Distinct from
# harshad (digit SUM). Self-checking: python replicates the exact three-branch
# message the kernel prints.
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
OUT="${TMPDIR:-/tmp}/isgapful.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 100 (IS, ->10), 108 (IS, ->18), 106 (NOT, ->16), 99 (out of range).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isgapful.png" \
    "echo warmup" "isgapful 100" "isgapful 108" "isgapful 106" "isgapful 99" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isgapful:|usage: isgapful" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):
    if n < 100:
        return f"isgapful: {n} is out of range (needs n >= 100)"
    first = int(str(n)[0])
    last = n % 10
    concat = first * 10 + last
    if n % concat == 0:
        return f"isgapful: {n} IS gapful (divisible by {concat})"
    return f"isgapful: {n} is NOT gapful (not divisible by {concat})"

for n in [100, 108, 106, 99]:
    want = message(n)
    if want not in s:
        print(f"ISGAPFUL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "isgapful: 100 IS gapful (divisible by 10)" in s
assert "isgapful: 108 IS gapful (divisible by 18)" in s
assert "isgapful: 106 is NOT gapful (not divisible by 16)" in s
assert "isgapful: 99 is out of range (needs n >= 100)" in s

print("ISGAPFUL TEST PASSED  (100->gapful(10), 108->gapful(18), 106->not(16), 99->out of range "
      "vs python; divisible by first+last digit concat)")
PY
