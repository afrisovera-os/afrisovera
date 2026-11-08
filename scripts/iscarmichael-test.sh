#!/usr/bin/env bash
# Verify the `iscarmichael <n>` command classifies Carmichael numbers via Korselt's
# criterion: n is composite, SQUAREFREE, and (p-1) | (n-1) for EVERY prime p | n.
# 561=3*11*17, 1105, 1729, 2465 ARE; 15=3*5 fails ((5-1)=4 does not divide 14); 9=3^2
# fails squarefree; primes fail composite. Distinct from isprime. Self-checking: python
# replicates Korselt's criterion via factorization.
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
OUT="${TMPDIR:-/tmp}/iscarmichael.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 561/1105 IS; 15 exercises the Korselt-fail reason; 9 the squarefree-fail reason.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iscarmichael.png" \
    "echo warmup" "iscarmichael 561" "iscarmichael 1105" "iscarmichael 15" "iscarmichael 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iscarmichael:|usage: iscarmichael" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def factor(n):
    f = {}
    x = n; d = 2
    while d * d <= x:
        while x % d == 0:
            f[d] = f.get(d, 0) + 1; x //= d
        d += 1
    if x > 1:
        f[x] = f.get(x, 0) + 1
    return f

def line(n):
    f = factor(n)
    squarefree = all(e == 1 for e in f.values())
    is_prime = len(f) == 1 and squarefree
    composite = not is_prime
    korselt = all((n - 1) % (p - 1) == 0 for p in f)
    if composite and squarefree and korselt:
        return f"iscarmichael: {n} IS a Carmichael number (squarefree composite; (p-1)|(n-1) for all p|n)"
    if is_prime:
        reason = "prime, not composite"
    elif not squarefree:
        reason = "not squarefree"
    else:
        reason = "(p-1) does not divide (n-1) for some prime factor p"
    return f"iscarmichael: {n} is NOT a Carmichael number ({reason})"

for n in [561, 1105, 15, 9]:
    want = line(n)
    if want not in s:
        print(f"ISCARMICHAEL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin all output shapes explicitly
assert "iscarmichael: 561 IS a Carmichael number (squarefree composite; (p-1)|(n-1) for all p|n)" in s
assert "iscarmichael: 1105 IS a Carmichael number (squarefree composite; (p-1)|(n-1) for all p|n)" in s
assert "iscarmichael: 15 is NOT a Carmichael number ((p-1) does not divide (n-1) for some prime factor p)" in s
assert "iscarmichael: 9 is NOT a Carmichael number (not squarefree)" in s

print("ISCARMICHAEL TEST PASSED  (561/1105 IS, 15 NOT (Korselt fail), 9 NOT (not squarefree) "
      "vs python; Korselt's criterion, distinct from isprime)")
PY
