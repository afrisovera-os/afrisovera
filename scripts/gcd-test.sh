#!/usr/bin/env bash
# Verify the `gcd` command computes GCD (Euclid) + LCM of two integers.
# Deterministic, so exact-substring assertions prove it:
#   gcd 48 36 -> "gcd(48, 36) = 12" and "lcm(48, 36) = 144"
#   gcd 17 5  -> "gcd(17, 5) = 1"   and "lcm(17, 5) = 85"  (coprime)
# Also verifies the LCM OVERFLOW GUARD: two large coprime inputs used to trigger
# a reachable KERNEL PANIC ("multiply with overflow") in the unguarded a/g*b; now
# the gcd line still prints and the lcm reports "overflows u64" -- and the shell
# survives (a follow-up `echo alive` still runs).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4702,server,nowait \
    -qmp tcp:127.0.0.1:4703,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/gcd.out"
python3 scripts/shell-demo.py 4702 4703 "${TMPDIR:-/tmp}/gcd.png" \
    "gcd 48 36" "gcd 17 5" "gcd 1000000000000 999999999999" "echo alive" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "gcd\(|lcm\(|alive" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["gcd(48, 36) = 12", "lcm(48, 36) = 144", "gcd(17, 5) = 1", "lcm(17, 5) = 85"]
missing = [w for w in want if w not in s]
if missing:
    print(f"GCD TEST FAILED (missing substring(s): {missing})"); sys.exit(1)

# Overflow guard: a=1000000000000, b=999999999999 are coprime, so
# lcm = a*b = ~1e24 >> u64 max -> must report overflow, NOT panic.
from math import gcd as _g
a, b = 1000000000000, 999999999999
assert _g(a, b) == 1 and a * b > 2**64 - 1, "overflow case assumption wrong"
if f"gcd({a}, {b}) = 1" not in s:
    print("GCD TEST FAILED (missing gcd line for the overflow case)"); sys.exit(1)
if f"lcm({a}, {b}) overflows u64" not in s:
    print("GCD TEST FAILED (lcm overflow not reported -- guard missing?)"); sys.exit(1)
# Prove the shell SURVIVED (no kernel panic): the follow-up command still ran.
if "KERNEL PANIC" in s:
    print("GCD TEST FAILED (kernel panicked on the overflow case)"); sys.exit(1)
if "alive" not in s:
    print("GCD TEST FAILED (shell did not survive the overflow case)"); sys.exit(1)

print("GCD TEST PASSED  (gcd/lcm 48,36 = 12/144; 17,5 = 1/85; huge coprime -> "
      "lcm overflows u64, no panic, shell survives)")
PY
