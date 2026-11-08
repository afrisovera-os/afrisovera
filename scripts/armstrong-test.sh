#!/usr/bin/env bash
# Verify the `armstrong <n>` command classifies whether n is an ARMSTRONG
# (narcissistic) number: the sum of each decimal digit raised to the power of the
# DIGIT COUNT equals n. e.g. 153 = 1^3+5^3+3^3, 9474 = 9^4+4^4+7^4+4^4. DISTINCT
# from digitsum (plain sum), digitprod (product), perfect (divisor sum).
# Self-checking: python recomputes d=len(str(n)); s=sum(int(c)**d for c in str(n))
# and asserts both the IS/NOT verdict and the printed power-sum.
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
OUT="${TMPDIR:-/tmp}/armstrong.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/armstrong.png" \
    "armstrong 153" "armstrong 9474" "armstrong 407" "armstrong 154" \
    "armstrong 10" "armstrong 0" "armstrong 5" "armstrong 200" "armstrong zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "armstrong:|usage: armstrong" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    d = len(str(n))
    tot = sum(int(c) ** d for c in str(n))
    verdict = "IS" if tot == n else "is NOT"
    return f"armstrong: {n} {verdict} an Armstrong number (sum of digits^{d} = {tot})"

# Known Armstrong: 153, 9474, 407, 0, 5 ; not: 154, 10, 200
for n in [153, 9474, 407, 154, 10, 0, 5, 200]:
    want = line(n)
    if want not in s:
        print(f"ARMSTRONG TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin a couple of exact sums and verdicts
assert line(153).endswith("(sum of digits^3 = 153)") and " IS " in line(153)
assert line(154).endswith("(sum of digits^3 = 190)") and " is NOT " in line(154)
assert " IS " in line(0) and " IS " in line(5)   # 0 and single digits are Armstrong

if "usage: armstrong <n>" not in s:
    print("ARMSTRONG TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("ARMSTRONG TEST PASSED  (153/9474/407/0/5 IS, 154/10/200 NOT, exact power-sums "
      "vs python, bad->usage)")
PY
