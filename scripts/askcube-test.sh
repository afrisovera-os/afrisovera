#!/usr/bin/env bash
# Verify the Asili assistant's NEW cube / cube-root math (docs/09 AI pillar), complementing the
# existing sqrt/squared: "ask 5 cubed"->125, "ask cube of 4"->64, "ask cube root of 27"->3,
# "ask cube root of 100"->4 (floor), "ask cbrt 64"->4. Integer, computed on-device. The cube-root
# branch is checked BEFORE the cube branch ("cube root" contains "cube"). Self-checking: python.
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
OUT="${TMPDIR:-/tmp}/askcube.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askcube.png" \
    "echo warmup" "ask 5 cubed" "ask cube of 4" "ask cube root of 27" \
    "ask cube root of 100" "ask cbrt 64" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def icbrt(n):
    r = 0
    while (r + 1) ** 3 <= n:
        r += 1
    return r

# derived expectations
assert f"[asili] = {5**3} (CUBED, ON-DEVICE)" in s          # 125
assert f"[asili] = {4**3} (CUBED, ON-DEVICE)" in s          # 64
assert f"[asili] = {icbrt(27)} (INTEGER CUBE ROOT, ON-DEVICE)" in s   # 3
assert f"[asili] = {icbrt(100)} (INTEGER CUBE ROOT, ON-DEVICE)" in s  # 4
assert f"[asili] = {icbrt(64)} (INTEGER CUBE ROOT, ON-DEVICE)" in s   # 4

print("ASKCUBE TEST PASSED  (5 cubed->125, cube of 4->64, cube root of 27->3, cube root of 100->4, "
      "cbrt 64->4; on-device Asili cube math)")
PY
