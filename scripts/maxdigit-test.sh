#!/usr/bin/env bash
# Verify the `maxdigit <n>` command reports both the largest and smallest
# decimal digit of n in one line (2468 -> max 8, min 2; 90210 -> max 9, min 0;
# 555 -> max 5, min 5; 0 -> max 0, min 0). Distinct from digitsum (SUM),
# digitcount (COUNT), and evendigits (even/odd split). Self-checking: python
# re-derives max/min over the digits.
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
OUT="${TMPDIR:-/tmp}/maxdigit.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/maxdigit.png" \
    "echo warmup" "maxdigit 2468" "maxdigit 90210" "maxdigit 555" \
    "maxdigit 0" "maxdigit zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "maxdigit:|usage: maxdigit" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    ds = [int(c) for c in str(n)]
    return f"maxdigit: {n} max {max(ds)}, min {min(ds)}"

for n in [2468, 90210, 555, 0]:
    want = line(n)
    if want not in s:
        print(f"MAXDIGIT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(2468) == "maxdigit: 2468 max 8, min 2"
assert line(90210) == "maxdigit: 90210 max 9, min 0"   # 0 is the min digit here
assert line(555) == "maxdigit: 555 max 5, min 5"        # all digits equal
assert line(0) == "maxdigit: 0 max 0, min 0"            # single digit 0

# a non-numeric arg -> usage
if "usage: maxdigit <n>" not in s:
    print("MAXDIGIT TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("MAXDIGIT TEST PASSED  (2468->8/2, 90210->9/0, 555->5/5, 0->0/0 vs python; "
      "bad -> usage)")
PY
