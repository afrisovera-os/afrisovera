#!/usr/bin/env bash
# Verify the Asili on-device assistant's NEW math (docs/09 AI pillar): integer square ROOTS
# and SQUARES. "ask sqrt 144" -> 12, "ask square root of 200" -> 14 (floor), "ask square of
# 12" -> 144, "ask 15 squared" -> 225, "ask sqrt 1000" -> 31 (floor). Computed on-device.
# Self-checking: python math.isqrt for roots, n*n for squares.
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
OUT="${TMPDIR:-/tmp}/asksqrt.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/asksqrt.png" \
    "echo warmup" "ask sqrt 144" "ask square root of 200" "ask square of 12" \
    "ask 15 squared" "ask sqrt 1000" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

# DERIVED references: floor sqrt and squares
sqrt144 = math.isqrt(144)   # 12
sqrt200 = math.isqrt(200)   # 14
sqr12   = 12 * 12           # 144
sqr15   = 15 * 15           # 225
sqrt1000 = math.isqrt(1000) # 31

checks = [
    f"[asili] = {sqrt144} (INTEGER SQRT, ON-DEVICE)",
    f"[asili] = {sqrt200} (INTEGER SQRT, ON-DEVICE)",
    f"[asili] = {sqr12} (SQUARED, ON-DEVICE)",
    f"[asili] = {sqr15} (SQUARED, ON-DEVICE)",
    f"[asili] = {sqrt1000} (INTEGER SQRT, ON-DEVICE)",
]
for want in checks:
    if want not in s:
        print(f"ASKSQRT TEST FAILED (missing '{want}')"); sys.exit(1)

assert "[asili] = 12 (INTEGER SQRT, ON-DEVICE)" in s
assert "[asili] = 14 (INTEGER SQRT, ON-DEVICE)" in s   # floor of sqrt(200)
assert "[asili] = 144 (SQUARED, ON-DEVICE)" in s
assert "[asili] = 225 (SQUARED, ON-DEVICE)" in s
assert "[asili] = 31 (INTEGER SQRT, ON-DEVICE)" in s

print("ASKSQRT TEST PASSED  (sqrt 144->12, sqrt 200->14 floor, square of 12->144, 15 squared->225, "
      "sqrt 1000->31 vs python isqrt / n*n, on-device Asili)")
PY
