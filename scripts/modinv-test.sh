#!/usr/bin/env bash
# Verify `modinv <a> <m>` computes the modular inverse (x with a*x=1 mod m) via extended
# Euclid: 3 11 -> 4, 10 17 -> 12, 7 26 -> 15, 1 5 -> 1; and reports "no inverse" when
# gcd(a,m) != 1 (2 4 -> gcd 2). Distinct from pow (mod exponentiation) and gcd. Self-checking:
# python's pow(a,-1,m) (and math.gcd for the no-inverse branch) is the reference.
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
OUT="${TMPDIR:-/tmp}/modinv.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/modinv.png" \
    "echo warmup" "modinv 3 11" "modinv 10 17" "modinv 7 26" \
    "modinv 1 5" "modinv 2 4" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "modinv:|usage: modinv" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(a, m):
    g = math.gcd(a % m, m)
    if g != 1:
        return f"modinv: no inverse of {a} mod {m} (gcd = {g})"
    return f"modinv: inverse of {a} mod {m} is {pow(a, -1, m)}"   # DERIVED by python

for a, m in [(3, 11), (10, 17), (7, 26), (1, 5), (2, 4)]:
    w = line(a, m)
    if w not in s:
        print(f"MODINV TEST FAILED (a={a} m={m} missing '{w}')"); sys.exit(1)

# pin the notable branches: classic inverse, larger modulus, coprime-with-26, identity, none
assert "modinv: inverse of 3 mod 11 is 4" in s
assert "modinv: inverse of 10 mod 17 is 12" in s
assert "modinv: inverse of 7 mod 26 is 15" in s
assert "modinv: inverse of 1 mod 5 is 1" in s
assert "modinv: no inverse of 2 mod 4 (gcd = 2)" in s

print("MODINV TEST PASSED  (3/11->4, 10/17->12, 7/26->15, 1/5->1, 2/4->no inverse "
      "vs python pow(a,-1,m))")
PY
