#!/usr/bin/env bash
# Verify the `iscircularprime <n>` command classifies circular primes: n is prime AND
# every cyclic rotation of its decimal digits is prime. 197 (197,971,719) and 13 (13,31)
# IS; 19 (91=7*13) and 4 (composite) NOT. Distinct from emirp and ispalprime.
# Self-checking: python replicates the rotate-all-and-prime-check rule.
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
OUT="${TMPDIR:-/tmp}/iscircularprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iscircularprime.png" \
    "echo warmup" "iscircularprime 197" "iscircularprime 13" "iscircularprime 19" "iscircularprime 4" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iscircularprime:|usage: iscircularprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(m):
    if m < 2: return False
    if m < 4: return True
    if m % 2 == 0: return False
    d = 3
    while d * d <= m:
        if m % d == 0: return False
        d += 2
    return True

def circular(n):  # mirror the kernel rule: every cyclic rotation must be prime
    t = str(n); d = len(t)
    return all(isp(int(t[k:] + t[:k])) for k in range(d))

for n in [197, 13, 19, 4]:
    want = f"iscircularprime: {n} IS a circular prime" if circular(n) \
        else f"iscircularprime: {n} is NOT a circular prime"
    if want not in s:
        print(f"ISCIRCULARPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "iscircularprime: 197 IS a circular prime" in s   # 197, 971, 719 all prime
assert "iscircularprime: 13 IS a circular prime" in s     # 13, 31 both prime
assert "iscircularprime: 19 is NOT a circular prime" in s # 91 = 7*13 composite
assert "iscircularprime: 4 is NOT a circular prime" in s  # not even prime

print("ISCIRCULARPRIME TEST PASSED  (197 IS (197,971,719), 13 IS (13,31), 19 NOT (91), "
      "4 NOT vs python rotate-all-prime rule)")
PY
