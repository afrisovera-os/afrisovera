#!/usr/bin/env bash
# Verify `cardinal <n>` spells a u64 in English cardinal words: 0->"zero", 123->"one
# hundred twenty-three", 1234567->"one million two hundred thirty-four thousand five
# hundred sixty-seven", and u64 max -> "eighteen quintillion ...". Distinct from roman/
# ordinal/numberwords. Self-checking: python builds every reference string (never typed).
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
OUT="${TMPDIR:-/tmp}/cardinal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/cardinal.png" \
    "echo warmup" "cardinal 0" "cardinal 123" "cardinal 1234567" "cardinal 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cardinal:|usage: cardinal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ONES = ["zero","one","two","three","four","five","six","seven","eight","nine","ten",
        "eleven","twelve","thirteen","fourteen","fifteen","sixteen","seventeen",
        "eighteen","nineteen"]
TENS = ["","","twenty","thirty","forty","fifty","sixty","seventy","eighty","ninety"]
SCALES = ["","thousand","million","billion","trillion","quadrillion","quintillion"]

def card(n):  # mirror the kernel spelling exactly
    if n == 0:
        return "zero"
    grp = []; x = n
    while x > 0:
        grp.append(x % 1000); x //= 1000
    out = []
    for gi in range(len(grp) - 1, -1, -1):
        g = grp[gi]
        if g == 0:
            continue
        seg = ""
        h, rem = g // 100, g % 100
        if h > 0:
            seg += ONES[h] + " hundred"
            if rem > 0:
                seg += " "
        if rem > 0:
            if rem < 20:
                seg += ONES[rem]
            else:
                seg += TENS[rem // 10]
                if rem % 10 > 0:
                    seg += "-" + ONES[rem % 10]
        if gi > 0:
            seg += " " + SCALES[gi]
        out.append(seg)
    return " ".join(out)

for n in [0, 123, 1234567, 18446744073709551615]:
    want = f"cardinal: {card(n)}"
    if want not in s:
        print(f"CARDINAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "cardinal: zero" in s
assert "cardinal: one hundred twenty-three" in s
assert "cardinal: one million two hundred thirty-four thousand five hundred sixty-seven" in s
assert ("cardinal: eighteen quintillion four hundred forty-six quadrillion seven hundred "
        "forty-four trillion seventy-three billion seven hundred nine million five hundred "
        "fifty-one thousand six hundred fifteen") in s

print("CARDINAL TEST PASSED  (0->zero, 123->one hundred twenty-three, 1234567 multi-scale, "
      "u64 max->eighteen quintillion.. vs python)")
PY
