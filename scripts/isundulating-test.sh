#!/usr/bin/env bash
# Verify the `isundulating <n>` command classifies undulating numbers: at least 3
# decimal digits that strictly alternate between two DIFFERENT values a,b,a,b,...
# (121, 1212, 70707, 2323 IS; 123, 1234 NOT (no alternation); 111 NOT (a==b);
# anything under 3 digits NOT). Distinct from isrepdigit (all EQUAL), isdigitsorted
# (non-decreasing), and maxdigit (extreme values). Self-checking: python builds the
# same alternation predicate.
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
OUT="${TMPDIR:-/tmp}/isundulating.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isundulating.png" \
    "echo warmup" "isundulating 1212" "isundulating 2323" "isundulating 111" "isundulating 99" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isundulating:|usage: isundulating" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isund(n):
    t = str(n)
    return len(t) >= 3 and t[0] != t[1] and all(t[i] == t[i - 2] for i in range(2, len(t)))

# IS cases
for n in [1212, 2323]:
    assert isund(n), f"python: {n} should be undulating"
    d = str(n)
    want = f"isundulating: {n} IS undulating (digits alternate {d[0]}{d[1]}{d[0]}...)"
    if want not in s:
        print(f"ISUNDULATING TEST FAILED (missing '{want}')"); sys.exit(1)

# NOT cases: 111 (a==b), 99 (fewer than 3 digits)
for n in [111, 99]:
    assert not isund(n)
    if f"isundulating: {n} is NOT undulating" not in s:
        print(f"ISUNDULATING TEST FAILED ({n} should be NOT)"); sys.exit(1)

# pin the notable cases explicitly
assert "isundulating: 1212 IS undulating (digits alternate 121...)" in s
assert "isundulating: 111 is NOT undulating" in s   # all-equal is not an undulation
assert "isundulating: 99 is NOT undulating" in s    # < 3 digits

print("ISUNDULATING TEST PASSED  (1212/2323 IS, 111 (a==b)/99 (<3 digits) NOT via "
      "digit-alternation walk vs python; distinct from isrepdigit/isdigitsorted)")
PY
