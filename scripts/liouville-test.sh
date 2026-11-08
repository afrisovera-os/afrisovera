#!/usr/bin/env bash
# Verify the `liouville <n>` command computes the Liouville function lambda(n) =
# (-1)^Omega(n) in {+1,-1}, where Omega(n) counts prime factors WITH multiplicity.
# lambda(1)=1, lambda(2)=-1, lambda(4)=+1 (2^2), lambda(6)=+1, lambda(8)=-1,
# lambda(12)=-1. The completely-multiplicative sibling of mobius: unlike mobius (0 on
# non-squarefree n), liouville counts with multiplicity so lambda(4)=+1 while mu(4)=0.
# Distinct from bigomega (the count). Self-checking: python replicates via Omega parity.
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
OUT="${TMPDIR:-/tmp}/liouville.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 4 pins the non-squarefree case (lambda=+1, contrasting mobius=0); 12 an odd-Omega case.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/liouville.png" \
    "echo warmup" "liouville 12" "liouville 4" "liouville 30" "liouville 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "liouville:|usage: liouville" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def bigomega(n):
    x = n; c = 0; d = 2
    while d * d <= x:
        while x % d == 0:
            x //= d; c += 1
        d += 1
    if x > 1:
        c += 1
    return c

def lam(n):
    return 1 if bigomega(n) % 2 == 0 else -1

for n in [12, 4, 30, 1]:
    want = f"liouville: lambda({n}) = {lam(n)}"
    if want not in s:
        print(f"LIOUVILLE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin both values + the mobius-contrasting non-squarefree case
assert "liouville: lambda(12) = -1" in s      # Omega=3 (2*2*3), odd
assert "liouville: lambda(4) = 1" in s        # Omega=2 (2*2), even -- mu(4)=0 but lambda(4)=+1
assert "liouville: lambda(30) = -1" in s      # Omega=3
assert "liouville: lambda(1) = 1" in s        # empty product

print("LIOUVILLE TEST PASSED  (lambda(12)=-1, lambda(4)=+1 (vs mu(4)=0), lambda(30)=-1, "
      "lambda(1)=1 vs python; (-1)^Omega, completely-multiplicative sibling of mobius)")
PY
