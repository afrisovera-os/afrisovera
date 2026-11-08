#!/usr/bin/env bash
# Verify `prevprime <n>` reports the largest prime STRICTLY LESS THAN n (mirror of nextprime):
# 10 -> 7, 3 -> 2, 100 -> 97, 20 -> 19, and n <= 2 -> "no prime below n". Self-checking:
# python's own trial-division previous-prime is the reference for every case.
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
OUT="${TMPDIR:-/tmp}/prevprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevprime.png" \
    "echo warmup" "prevprime 10" "prevprime 3" "prevprime 100" \
    "prevprime 20" "prevprime 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevprime:|usage: prevprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(k):
    if k < 2: return False
    if k == 2: return True
    if k % 2 == 0: return False
    d = 3
    while d * d <= k:
        if k % d == 0: return False
        d += 2
    return True

def line(n):
    if n <= 2:
        return f"prevprime: no prime below {n}"
    c = n - 1
    while c >= 2 and not isp(c):
        c -= 1
    return f"prevprime: prev prime before {n} is {c}"    # DERIVED, never hand-typed

for n in [10, 3, 100, 20, 2]:
    w = line(n)
    if w not in s:
        print(f"PREVPRIME TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: composite scan-down, adjacent prime, larger n, none-below-2
assert "prevprime: prev prime before 10 is 7" in s
assert "prevprime: prev prime before 3 is 2" in s
assert "prevprime: prev prime before 100 is 97" in s
assert "prevprime: prev prime before 20 is 19" in s
assert "prevprime: no prime below 2" in s

print("PREVPRIME TEST PASSED  (10->7, 3->2, 100->97, 20->19, 2->none "
      "vs python trial-division previous-prime)")
PY
