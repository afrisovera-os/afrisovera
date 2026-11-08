#!/usr/bin/env bash
# Verify the `ismunchausen <n>` command classifies Munchausen numbers: n equals
# the sum of each of its decimal digits raised to ITSELF (d^d), with 0^0 = 0. The
# only base-10 Munchausen numbers are 0, 1, 3435 (=3^3+4^4+3^3+5^5) and 438579088.
# Uses a fixed 0..=9 d^d table. Distinct from armstrong (digit^digit-count),
# isdisarium (digit^position), and isfactorion (digit factorials). Self-checking:
# python sum((0 if int(d)==0 else int(d)**int(d)) for d in str(n)).
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
OUT="${TMPDIR:-/tmp}/ismunchausen.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 438579088 (the largest base-10 Munchausen number) exercises the u128 accumulation.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ismunchausen.png" \
    "echo warmup" "ismunchausen 3435" "ismunchausen 438579088" "ismunchausen 5" "ismunchausen 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ismunchausen:|usage: ismunchausen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ddsum(n):
    return sum((0 if int(d) == 0 else int(d) ** int(d)) for d in str(n))

# (n) -> expected line built from the digit^digit sum and whether it equals n
for n in [3435, 438579088, 5, 1]:
    tot = ddsum(n)
    verb = "IS" if tot == n else "is NOT"
    want = f"ismunchausen: {n} {verb} a Munchausen number (sum of digit^digit = {tot})"
    if want not in s:
        print(f"ISMUNCHAUSEN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "ismunchausen: 3435 IS a Munchausen number (sum of digit^digit = 3435)" in s
assert "ismunchausen: 438579088 IS a Munchausen number (sum of digit^digit = 438579088)" in s
assert "ismunchausen: 5 is NOT a Munchausen number (sum of digit^digit = 3125)" in s  # 5^5 = 3125

print("ISMUNCHAUSEN TEST PASSED  (1/3435/438579088 IS, 5 NOT (5^5=3125) via digit^digit "
      "sum vs python; distinct from armstrong/isdisarium/isfactorion)")
PY
