#!/usr/bin/env bash
# Verify the `ispolydivisible <n>` command classifies polydivisible numbers: every k-digit
# prefix of n is divisible by k. 1200 (1|1,12|2,120|3,1200|4) and 123 (1,12/2,123/3) IS;
# 15 and 47 NOT (first 2 digits not divisible by 2). Distinct from harshad/isdigitsorted.
# Self-checking: python replicates the prefix-divisibility rule incl. the failing k.
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
OUT="${TMPDIR:-/tmp}/ispolydivisible.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ispolydivisible.png" \
    "echo warmup" "ispolydivisible 1200" "ispolydivisible 123" "ispolydivisible 15" "ispolydivisible 47" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ispolydivisible:|usage: ispolydivisible" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):  # mirror the kernel: every k-digit prefix divisible by k
    t = str(n)
    for i in range(len(t)):
        if int(t[:i + 1]) % (i + 1) != 0:
            k = i + 1
            return f"ispolydivisible: {n} is NOT polydivisible (first {k} digits not divisible by {k})"
    return f"ispolydivisible: {n} IS polydivisible"

for n in [1200, 123, 15, 47]:
    want = message(n)
    if want not in s:
        print(f"ISPOLYDIVISIBLE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable branches explicitly (derived above)
assert "ispolydivisible: 1200 IS polydivisible" in s
assert "ispolydivisible: 123 IS polydivisible" in s
assert "ispolydivisible: 15 is NOT polydivisible (first 2 digits not divisible by 2)" in s
assert "ispolydivisible: 47 is NOT polydivisible (first 2 digits not divisible by 2)" in s

print("ISPOLYDIVISIBLE TEST PASSED  (1200 IS, 123 IS, 15 NOT (first 2), 47 NOT (first 2) "
      "vs python prefix-divisibility rule)")
PY
