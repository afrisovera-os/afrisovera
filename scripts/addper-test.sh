#!/usr/bin/env bash
# Verify the `addper <n>` command reports the ADDITIVE PERSISTENCE of n: how many
# repeated digit-SUM steps reduce n to a single digit (7->0; 39->12->3 = 2;
# 199->19->10->1 = 3; 9999->36->9 = 2; single digits incl 0 -> 0), plus the
# final digit it reaches. The SUM analog of digitprod's multiplicative
# persistence; digitsum prints the digital ROOT but NOT the step count. Self-
# checking: python re-derives the step count and final digit.
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
OUT="${TMPDIR:-/tmp}/addper.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/addper.png" \
    "echo warmup" "addper 7" "addper 39" "addper 199" "addper 9999" \
    "addper 0" "addper zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "addper:|usage: addper" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dsum(n):
    return sum(int(c) for c in str(n))

def addper(n):
    steps = 0
    x = n
    while x >= 10:
        x = dsum(x)
        steps += 1
    return steps, x

def line(n):
    steps, root = addper(n)
    return f"addper: {n} has additive persistence {steps} (reaches {root})"

for n in [7, 39, 199, 9999, 0]:
    want = line(n)
    if want not in s:
        print(f"ADDPER TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(7) == "addper: 7 has additive persistence 0 (reaches 7)"      # single digit
assert line(39) == "addper: 39 has additive persistence 2 (reaches 3)"    # 39->12->3
assert line(199) == "addper: 199 has additive persistence 3 (reaches 1)"  # 199->19->10->1
assert line(9999) == "addper: 9999 has additive persistence 2 (reaches 9)"  # 9999->36->9

# a non-numeric arg -> usage
if "usage: addper <n>" not in s:
    print("ADDPER TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("ADDPER TEST PASSED  (7->0, 39->2, 199->3, 9999->2 vs python; the SUM analog of "
      "digitprod's persistence; bad -> usage)")
PY
