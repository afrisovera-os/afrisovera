#!/usr/bin/env bash
# Verify the `altdigitsum <n>` command computes the alternating digit sum from the
# right (d0 - d1 + d2 - ...) and reports divisibility by 11 (121 -> 0 divisible;
# 2728 -> 11 divisible; 1234 -> 2 not). Distinct from digitsum (plain sum + root).
# Self-checking: python replicates the alternating sum + the % 11 verdict.
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
OUT="${TMPDIR:-/tmp}/altdigitsum.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 121 (sum 0, div), 2728 (sum 11, div), 1234 (sum 2, not), 0 (sum 0, div).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/altdigitsum.png" \
    "echo warmup" "altdigitsum 121" "altdigitsum 2728" "altdigitsum 1234" "altdigitsum 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "altdigitsum:|usage: altdigitsum" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def altsum(n):
    total = 0
    sign = 1
    x = n
    while x > 0:
        total += sign * (x % 10)
        sign = -sign
        x //= 10
    return total

def message(n):
    a = altsum(n)
    d = "divisible by 11" if a % 11 == 0 else "not divisible by 11"
    return f"altdigitsum: {n} has alternating digit sum {a} ({d})"

for n in [121, 2728, 1234, 0]:
    want = message(n)   # alternating sum + verdict DERIVED by python, never hand-typed
    if want not in s:
        print(f"ALTDIGITSUM TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "altdigitsum: 121 has alternating digit sum 0 (divisible by 11)" in s
assert "altdigitsum: 2728 has alternating digit sum 11 (divisible by 11)" in s  # sum != 0 but multiple of 11
assert "altdigitsum: 1234 has alternating digit sum 2 (not divisible by 11)" in s

print("ALTDIGITSUM TEST PASSED  (121->0 div, 2728->11 div, 1234->2 not, 0->0 div vs python; "
      "alternating digit sum + divisibility by 11)")
PY
