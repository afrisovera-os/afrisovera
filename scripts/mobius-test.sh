#!/usr/bin/env bash
# Verify the `mobius <n>` command computes the Mobius function mu(n) in {-1, 0, +1}:
# mu(1)=1; mu(n)=0 when a squared prime divides n (not squarefree); otherwise
# mu(n)=(-1)^k for k distinct primes. mu(1)=1, mu(2)=-1, mu(6)=+1, mu(30)=-1, mu(4)=0,
# mu(12)=0. Combines the squarefree test with omega (distinct-prime count); distinct
# from squarefree (yes/no), omega (a count), radical. Self-checking: python replicates.
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
OUT="${TMPDIR:-/tmp}/mobius.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 30 -> -1 (odd # primes), 6 -> +1 (even), 12 -> 0 (2^2 divides), 1 -> +1 (empty product).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mobius.png" \
    "echo warmup" "mobius 30" "mobius 6" "mobius 12" "mobius 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mobius:|usage: mobius" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def mu(n):
    x = n; distinct = 0; squarefree = True; d = 2
    while d * d <= x:
        if x % d == 0:
            distinct += 1; e = 0
            while x % d == 0:
                x //= d; e += 1
            if e >= 2:
                squarefree = False
                break
        d += 1
    if x > 1:
        distinct += 1
    if not squarefree:
        return 0
    return 1 if distinct % 2 == 0 else -1

for n in [30, 6, 12, 1]:
    want = f"mobius: mu({n}) = {mu(n)}"
    if want not in s:
        print(f"MOBIUS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin all three values explicitly
assert "mobius: mu(30) = -1" in s      # 2*3*5, 3 distinct primes (odd)
assert "mobius: mu(6) = 1" in s        # 2*3, 2 distinct primes (even)
assert "mobius: mu(12) = 0" in s       # 2^2*3, not squarefree
assert "mobius: mu(1) = 1" in s        # empty product

print("MOBIUS TEST PASSED  (mu(30)=-1, mu(6)=+1, mu(12)=0, mu(1)=1 vs python; "
      "squarefree + distinct-prime parity into {-1,0,+1}, distinct from squarefree/omega)")
PY
