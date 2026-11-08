#!/usr/bin/env bash
# Verify `prevcomposite <n>` gives the largest composite < n (composites 4,6,8,9,10..): 10 ->
# 9, 5 -> 4, 13 -> 12, 100 -> 99, and n <= 4 -> "no composite below n". Composite mirror of
# prevprime. Self-checking: python scans n-1 downward for the first composite.
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
OUT="${TMPDIR:-/tmp}/prevcomposite.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevcomposite.png" \
    "echo warmup" "prevcomposite 10" "prevcomposite 5" "prevcomposite 13" \
    "prevcomposite 100" "prevcomposite 4" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevcomposite:|usage: prevcomposite" "$OUT" | sed 's/^/  /'

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

def line(n):
    if n <= 4:
        return f"prevcomposite: no composite below {n}"
    c = n - 1
    while c >= 4 and isp(c):        # skip primes down to the first composite
        c -= 1
    return f"prevcomposite: prev composite before {n} is {c}"

for n in [10, 5, 13, 100, 4]:
    w = line(n)
    if w not in s:
        print(f"PREVCOMPOSITE TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

assert "prevcomposite: prev composite before 10 is 9" in s
assert "prevcomposite: prev composite before 5 is 4" in s
assert "prevcomposite: prev composite before 13 is 12" in s
assert "prevcomposite: prev composite before 100 is 99" in s
assert "prevcomposite: no composite below 4" in s

print("PREVCOMPOSITE TEST PASSED  (10->9, 5->4, 13->12, 100->99, 4->none vs python composite scan)")
PY
