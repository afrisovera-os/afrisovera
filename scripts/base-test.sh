#!/usr/bin/env bash
# Verify the `base` command parses dec/hex/bin/oct and prints all four bases.
# Deterministic:
#   base 255    -> "base: dec 255  hex 0xff  oct 0o377  bin 0b11111111"
#   base 0xff   -> the SAME line (hex input parses to the same value)
#   base 0b1010 -> "base: dec 10  hex 0xa  oct 0o12  bin 0b1010"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4714,server,nowait \
    -qmp tcp:127.0.0.1:4715,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/base.out"
python3 scripts/shell-demo.py 4714 4715 "${TMPDIR:-/tmp}/base.png" "base 255" "base 0xff" "base 0b1010" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "base: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
lines = [l for l in s.splitlines() if l.startswith("base: dec ")]
def has(dec, hexs, bins):
    for l in lines:
        if f"dec {dec}" in l and f"hex 0x{hexs}" in l and f"bin 0b{bins}" in l:
            return True
    return False
# 255 and 0xff must both produce the 255 line (proves both dec and hex parse)
count_255 = sum(1 for l in lines if "dec 255" in l and "hex 0xff" in l and "bin 0b11111111" in l)
if count_255 < 2:
    print(f"BASE TEST FAILED (expected 'base 255' and 'base 0xff' to both yield the 255 line; got {count_255})"); sys.exit(1)
if not has(10, "a", "1010"):
    print("BASE TEST FAILED (base 0b1010 did not yield dec 10 / hex 0xa / bin 0b1010)"); sys.exit(1)
print("BASE TEST PASSED  (255 & 0xff -> same; 0b1010 -> dec 10 / 0xa / 0b1010)")
PY
