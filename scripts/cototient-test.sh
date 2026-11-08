#!/usr/bin/env bash
# Verify `cototient <n>` computes the Euler cototient co-phi(n) = n - phi(n): the count of
# integers in 1..=n that SHARE a factor with n (NOT coprime). OEIS A051953. co-phi(prime)=1,
# co-phi(1)=0, co-phi(12)=8, co-phi(9)=3. The complement of totient. Self-checking: python
# computes phi(n) by the same sqrt-time factorization the kernel uses.
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
OUT="${TMPDIR:-/tmp}/cototient.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/cototient.png" \
    "echo warmup" "cototient 1" "cototient 13" "cototient 12" "cototient 360" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cototient:|usage: cototient" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def phi(n):  # mirror the kernel: sqrt-time prime-factorization product
    result = n; m = n; p = 2
    while p * p <= m:
        if m % p == 0:
            while m % p == 0:
                m //= p
            result -= result // p
        p += 1
    if m > 1:
        result -= result // m
    return result

for n in [1, 13, 12, 360]:
    ph = phi(n)
    want = f"cototient: cophi({n}) = {n} - {ph} = {n - ph}"  # DERIVED, never hand-typed
    if want not in s:
        print(f"COTOTIENT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable branches: 1->0, prime 13->1, 12->8, 360->264
assert "cototient: cophi(1) = 1 - 1 = 0" in s
assert "cototient: cophi(13) = 13 - 12 = 1" in s
assert "cototient: cophi(12) = 12 - 4 = 8" in s
assert "cototient: cophi(360) = 360 - 96 = 264" in s

print("COTOTIENT TEST PASSED  (co-phi(1)=0, co-phi(13)=1, co-phi(12)=8, co-phi(360)=264 "
      "vs python n - phi(n))")
PY
