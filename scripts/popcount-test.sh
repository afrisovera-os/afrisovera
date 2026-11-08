#!/usr/bin/env bash
# Verify the `popcount` command reports set-bit count + leading/trailing zeros.
# Deterministic:
#   popcount 255  -> 0xff = 8 set bits
#   popcount 1024 -> 2^10 = 1 set bit, 10 trailing zeros
#   popcount 0    -> 0 set bits
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4742,server,nowait \
    -qmp tcp:127.0.0.1:4743,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/popcount.out"
python3 scripts/shell-demo.py 4742 4743 "${TMPDIR:-/tmp}/popcount.png" "popcount 255" "popcount 1024" "popcount 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "popcount: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
def line_for(n):
    for l in s.splitlines():
        if l.startswith(f"popcount: {n} "):
            return l
    return ""
l255, l1024, l0 = line_for(255), line_for(1024), line_for(0)
if "8 set bits" not in l255:
    print(f"POPCOUNT TEST FAILED (255 -> {l255!r}; expected 8 set bits)"); sys.exit(1)
if "1 set bits" not in l1024 or "10 trailing zeros" not in l1024:
    print(f"POPCOUNT TEST FAILED (1024 -> {l1024!r}; expected 1 set bit + 10 trailing zeros)"); sys.exit(1)
if "0 set bits" not in l0:
    print(f"POPCOUNT TEST FAILED (0 -> {l0!r}; expected 0 set bits)"); sys.exit(1)
print("POPCOUNT TEST PASSED  (255->8 bits; 1024->1 bit/10 tz; 0->0 bits)")
PY
