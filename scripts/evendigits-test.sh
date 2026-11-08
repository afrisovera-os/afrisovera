#!/usr/bin/env bash
# Verify the `evendigits <n>` command counts how many of n's DECIMAL digits are
# EVEN vs ODD (0,2,4,6,8 vs 1,3,5,7,9): 2468 -> 4 even/0 odd; 1357 -> 0/4;
# 1234567890 -> 5/5; 0 -> 1 even/0 odd (the single digit 0). Distinct from
# digitcount (TOTAL digit count), digitsum, and digitprod. Self-checking: python
# re-derives the even/odd split.
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
OUT="${TMPDIR:-/tmp}/evendigits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/evendigits.png" \
    "echo warmup" "evendigits 2468" "evendigits 1357" "evendigits 1234567890" \
    "evendigits 0" "evendigits zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "evendigits:|usage: evendigits" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    d = str(n)
    ev = sum(1 for c in d if int(c) % 2 == 0)
    od = len(d) - ev
    return f"evendigits: {n} has {ev} even, {od} odd"

for n in [2468, 1357, 1234567890, 0]:
    want = line(n)
    if want not in s:
        print(f"EVENDIGITS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(2468) == "evendigits: 2468 has 4 even, 0 odd"
assert line(1357) == "evendigits: 1357 has 0 even, 4 odd"
assert line(1234567890) == "evendigits: 1234567890 has 5 even, 5 odd"
assert line(0) == "evendigits: 0 has 1 even, 0 odd"    # 0 is a single even digit

# a non-numeric arg -> usage
if "usage: evendigits <n>" not in s:
    print("EVENDIGITS TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("EVENDIGITS TEST PASSED  (2468->4/0, 1357->0/4, 1234567890->5/5, 0->1/0 vs python; "
      "bad -> usage)")
PY
