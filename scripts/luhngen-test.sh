#!/usr/bin/env bash
# Verify `luhngen <digits>` computes the Luhn check digit for a payload: 7992739871 -> 3,
# 4 -> 2, 0 -> 0, 123456789 -> 7, 111 -> 5. Whereas luhn validates, luhngen generates the
# missing digit. Self-checking: python computes the same check digit AND asserts that
# appending it makes the full number Luhn-valid.
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
OUT="${TMPDIR:-/tmp}/luhngen.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/luhngen.png" \
    "echo warmup" "luhngen 7992739871" "luhngen 4" "luhngen 0" \
    "luhngen 123456789" "luhngen 111" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "luhngen:|usage: luhngen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def luhngen(payload):
    tot = 0
    for i, ch in enumerate(reversed(payload)):
        v = int(ch)
        if i % 2 == 0:
            v *= 2
            if v > 9: v -= 9
        tot += v
    return (10 - tot % 10) % 10

def luhn_valid(num):                      # cross-check the generated full number
    tot = 0
    for i, ch in enumerate(reversed(num)):
        v = int(ch)
        if i % 2 == 1:
            v *= 2
            if v > 9: v -= 9
        tot += v
    return tot % 10 == 0

for payload in ["7992739871", "4", "0", "123456789", "111"]:
    d = luhngen(payload)
    assert luhn_valid(payload + str(d)), f"{payload}+{d} not Luhn-valid"   # independent check
    want = f"luhngen: check digit for {payload} is {d} ({payload}{d})"
    if want not in s:
        print(f"LUHNGEN TEST FAILED (payload {payload} missing '{want}')"); sys.exit(1)

# pin the notable cases
assert "luhngen: check digit for 7992739871 is 3 (79927398713)" in s
assert "luhngen: check digit for 4 is 2 (42)" in s
assert "luhngen: check digit for 0 is 0 (00)" in s
assert "luhngen: check digit for 123456789 is 7 (1234567897)" in s
assert "luhngen: check digit for 111 is 5 (1115)" in s

print("LUHNGEN TEST PASSED  (7992739871->3, 4->2, 0->0, 123456789->7, 111->5; each full "
      "number verified Luhn-valid)")
PY
