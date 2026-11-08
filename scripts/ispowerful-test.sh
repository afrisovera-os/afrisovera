#!/usr/bin/env bash
# Verify the `ispowerful <n>` command classifies powerful numbers: every prime
# factor appears with exponent >= 2 (1, 4, 8, 9, 16, 25, 27, 36, 72, 144 IS; 2, 3,
# 12=2^2*3 NOT). The natural OPPOSITE of squarefree (every exponent == 1). 1 is
# vacuously powerful. Distinct from squarefree/semiprime/radical. Self-checking:
# python trial-division exponent check.
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
OUT="${TMPDIR:-/tmp}/ispowerful.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 1 pins the vacuous case; 12 = 2^2*3 pins the exponent-1-on-3 rejection.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ispowerful.png" \
    "echo warmup" "ispowerful 72" "ispowerful 12" "ispowerful 1" "ispowerful 800" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ispowerful:|usage: ispowerful" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def powerful(n):
    if n < 1: return False
    x, d = n, 2
    while d * d <= x:
        if x % d == 0:
            e = 0
            while x % d == 0: x //= d; e += 1
            if e < 2: return False
        d += 1
    return x <= 1  # leftover prime has exponent 1

for n in [72, 12, 1, 800]:
    verb = "IS" if powerful(n) else "is NOT"
    tail = "(every prime factor has exponent >= 2)" if powerful(n) else "(a prime factor has exponent 1)"
    want = f"ispowerful: {n} {verb} powerful {tail}"
    if want not in s:
        print(f"ISPOWERFUL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "ispowerful: 72 IS powerful (every prime factor has exponent >= 2)" in s   # 2^3*3^2
assert "ispowerful: 12 is NOT powerful (a prime factor has exponent 1)" in s       # 2^2*3^1
assert "ispowerful: 1 IS powerful (every prime factor has exponent >= 2)" in s     # vacuous
assert "ispowerful: 800 IS powerful (every prime factor has exponent >= 2)" in s   # 2^5*5^2

print("ISPOWERFUL TEST PASSED  (72/1/800 IS, 12 (2^2*3) NOT via prime-exponent check "
      "vs python; the opposite of squarefree; distinct from semiprime/radical)")
PY
