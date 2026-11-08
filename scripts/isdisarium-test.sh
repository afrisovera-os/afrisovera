#!/usr/bin/env bash
# Verify the `isdisarium <n>` command classifies Disarium numbers: n equals the
# sum of each decimal digit raised to the power of its 1-based POSITION from the
# left (89 = 8^1+9^2, 135 = 1^1+3^2+5^3, 175 = 1^1+7^2+5^3). Distinct from
# armstrong, which raises every digit to the SAME power (the digit count) --
# 153 IS an Armstrong number but is NOT Disarium (1^1+5^2+3^3 = 53). Self-
# checking: python computes the position-power sum for each case.
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
OUT="${TMPDIR:-/tmp}/isdisarium.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isdisarium.png" \
    "echo warmup" "isdisarium 135" "isdisarium 175" "isdisarium 153" "isdisarium 598" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isdisarium:|usage: isdisarium" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def pos_sum(n):
    return sum(int(d) ** i for i, d in enumerate(str(n), 1))

# (n, expected_disarium?)
cases = [(135, True), (175, True), (153, False), (598, True)]
for n, want_is in cases:
    tot = pos_sum(n)
    verb = "IS" if (tot == n) else "is NOT"
    assert (tot == n) == want_is, f"python disagrees on {n}"
    want = f"isdisarium: {n} {verb} a Disarium number (sum of digit^position = {tot})"
    if want not in s:
        print(f"ISDISARIUM TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the contrast with armstrong: 153 is a classic Armstrong number (1^3+5^3+3^3)
# yet is NOT Disarium (positions differ): 1^1+5^2+3^3 = 1+25+27 = 53.
assert "isdisarium: 153 is NOT a Disarium number (sum of digit^position = 53)" in s

print("ISDISARIUM TEST PASSED  (135/175/598 IS, 153 NOT via position-power sum vs "
      "python; distinct from armstrong's fixed-power narcissistic test)")
PY
