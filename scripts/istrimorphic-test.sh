#!/usr/bin/env bash
# Verify the `istrimorphic <n>` command classifies whether n is a TRIMORPHIC
# number: n^3 ends in the digits of n, i.e. n^3 mod 10^k == n where k is the
# digit count of n (1,4,5,6,9,24,25,49,51,75,76,99,125,... yes). The CUBE analog
# of automorphic (n^2 ends in n). Cubing is u128-guarded: a very large n reports
# "too large to cube in u128" rather than panicking (the 2.265 lesson).
# Self-checking: python computes n**3 % 10**len(str(n)) == n.
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
OUT="${TMPDIR:-/tmp}/istrimorphic.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# The 7e12 case exercises the u128 overflow guard, then `echo alive` proves the
# shell survived (no kernel panic).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/istrimorphic.png" \
    "echo warmup" "istrimorphic 24" "istrimorphic 51" "istrimorphic 7" \
    "istrimorphic 7000000000000" "echo alive" "istrimorphic" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "istrimorphic:|usage: istrimorphic|alive" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def is_tri(n):
    k = len(str(n))
    return (n ** 3) % (10 ** k) == n

def line(n):
    if is_tri(n):
        return f"istrimorphic: {n} IS trimorphic ({n}^3 = {n**3} ends in {n})"
    return f"istrimorphic: {n} is NOT trimorphic ({n}^3 = {n**3})"

for n in [24, 51, 7]:
    want = line(n)
    if want not in s:
        print(f"ISTRIMORPHIC TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases (with the cube arithmetic shown)
assert line(24) == "istrimorphic: 24 IS trimorphic (24^3 = 13824 ends in 24)"
assert line(51) == "istrimorphic: 51 IS trimorphic (51^3 = 132651 ends in 51)"
assert line(7) == "istrimorphic: 7 is NOT trimorphic (7^3 = 343)"

# u128 overflow guard: 7e12 cubed exceeds u128, so it must report the guard
# message AND the shell must survive (echo alive runs after).
if "istrimorphic: 7000000000000 too large to cube in u128" not in s:
    print("ISTRIMORPHIC TEST FAILED (overflow guard message missing)"); sys.exit(1)
if "alive" not in s:
    print("ISTRIMORPHIC TEST FAILED (shell did not survive the overflow case)"); sys.exit(1)

# a non-numeric arg -> usage
if "usage: istrimorphic <n>" not in s:
    print("ISTRIMORPHIC TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("ISTRIMORPHIC TEST PASSED  (24/51 yes, 7 no vs python; 7e12 -> overflow-guarded, "
      "shell survives; bad -> usage)")
PY
