#!/usr/bin/env bash
# Verify `issemiperfect <n>` classifies whether n is a SEMIPERFECT (pseudoperfect) number:
# n equals the sum of some SUBSET of its own proper divisors. 6=1+2+3, 12=2+4+6 ARE (every
# PERFECT number is trivially semiperfect via its full divisor set); 8 (divisors 1,2,4, max
# reachable 7) is NOT. 70 is the classic WEIRD number -- ABUNDANT (sigma-n=74-70=4>0) yet NOT
# semiperfect -- the key case that distinguishes this from a plain abundant/deficient check.
# Self-checking: python re-derives via an independent subset-sum DP over proper divisors.
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
OUT="${TMPDIR:-/tmp}/issemiperfect.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/issemiperfect.png" \
    "echo warmup" "issemiperfect 6" "issemiperfect 12" "issemiperfect 8" \
    "issemiperfect 70" "issemiperfect 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "issemiperfect:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def proper_divisors(n):
    divs = []
    d = 1
    while d * d <= n:
        if n % d == 0:
            if d != n:
                divs.append(d)
            e = n // d
            if e != d and e != n:
                divs.append(e)
        d += 1
    return divs

def is_semiperfect(n):
    reachable = {0}
    for d in proper_divisors(n):
        reachable |= {s + d for s in reachable if s + d <= n}
    return n in reachable

def line(n):
    verdict = "IS" if is_semiperfect(n) else "is NOT"
    return f"issemiperfect: {n} {verdict} semiperfect"

for n in [6, 12, 8, 70]:
    want = line(n)
    if want not in s:
        print(f"ISSEMIPERFECT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(6) == "issemiperfect: 6 IS semiperfect"     # perfect number, full divisor set
assert line(12) == "issemiperfect: 12 IS semiperfect"   # 2+4+6
assert line(8) == "issemiperfect: 8 is NOT semiperfect" # divisors 1,2,4 max out at 7
assert line(70) == "issemiperfect: 70 is NOT semiperfect"  # WEIRD: abundant but not semiperfect

# n=0 must hit the explicit zero branch, not a verdict
if "issemiperfect: 0 is NOT semiperfect (needs n >= 1)" not in s:
    print("ISSEMIPERFECT TEST FAILED (n=0 should report the needs-n>=1 message)"); sys.exit(1)

print("ISSEMIPERFECT TEST PASSED  (6 & 12 semiperfect, 8 not, 70 weird-not-semiperfect, "
      "n=0 -> guard) vs python")
PY
