#!/usr/bin/env bash
# Verify `nextcomposite <n>` gives the smallest composite > n (composites 4,6,8,9,10..):
# 3 -> 4, 4 -> 6, 8 -> 9, 1 -> 4, 13 -> 14. Composite mirror of nextprime. Self-checking:
# python scans n+1 upward for the first composite (>1 and not prime).
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
OUT="${TMPDIR:-/tmp}/nextcomposite.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextcomposite.png" \
    "echo warmup" "nextcomposite 3" "nextcomposite 4" "nextcomposite 8" \
    "nextcomposite 1" "nextcomposite 13" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextcomposite:|usage: nextcomposite" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(k):
    if k < 2: return False
    if k % 2 == 0: return k == 2
    d = 3
    while d * d <= k:
        if k % d == 0: return False
        d += 2
    return True

def nextcomp(n):
    c = n + 1
    while c < 4 or isp(c):        # skip 1,2,3 and all primes
        c += 1
    return c

for n in [3, 4, 8, 1, 13]:
    want = f"nextcomposite: next composite after {n} is {nextcomp(n)}"    # DERIVED
    if want not in s:
        print(f"NEXTCOMPOSITE TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "nextcomposite: next composite after 3 is 4" in s
assert "nextcomposite: next composite after 4 is 6" in s
assert "nextcomposite: next composite after 8 is 9" in s
assert "nextcomposite: next composite after 1 is 4" in s
assert "nextcomposite: next composite after 13 is 14" in s

print("NEXTCOMPOSITE TEST PASSED  (3->4, 4->6, 8->9, 1->4, 13->14 vs python composite scan)")
PY
