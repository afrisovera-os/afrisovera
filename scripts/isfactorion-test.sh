#!/usr/bin/env bash
# Verify the `isfactorion <n>` command classifies factorions: n equals the sum of
# the factorials of its decimal digits. The only base-10 factorions are 1 (=1!),
# 2 (=2!), 145 (=1!+4!+5!), and 40585 (=4!+0!+5!+8!+5!). Uses a fixed 0..=9
# factorial table (0! = 1). Distinct from armstrong (sum of digit^digit-count) and
# isdisarium (sum of digit^position) -- this sums FACTORIALS, not powers. Self-
# checking: python sum(math.factorial(int(d)) for d in str(n)).
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
OUT="${TMPDIR:-/tmp}/isfactorion.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isfactorion.png" \
    "echo warmup" "isfactorion 145" "isfactorion 40585" "isfactorion 3" "isfactorion 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isfactorion:|usage: isfactorion" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def facsum(n):
    return sum(math.factorial(int(d)) for d in str(n))

# (n) -> expected line built from the digit-factorial sum and whether it equals n
for n in [145, 40585, 3, 1]:
    tot = facsum(n)
    verb = "IS" if tot == n else "is NOT"
    want = f"isfactorion: {n} {verb} a factorion (sum of digit factorials = {tot})"
    if want not in s:
        print(f"ISFACTORION TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "isfactorion: 145 IS a factorion (sum of digit factorials = 145)" in s
assert "isfactorion: 40585 IS a factorion (sum of digit factorials = 40585)" in s
assert "isfactorion: 3 is NOT a factorion (sum of digit factorials = 6)" in s  # 3! = 6 != 3

print("ISFACTORION TEST PASSED  (1/145/40585 IS, 3 NOT (3!=6) via digit-factorial "
      "sum vs python; distinct from armstrong/isdisarium which sum POWERS)")
PY
