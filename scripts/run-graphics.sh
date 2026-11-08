#!/usr/bin/env bash
# Boot Mansa with a framebuffer and capture the screen to a PNG (headless).
# Needs: qemu-system-aarch64 on PATH, python3 for the QMP screendump.
#
#   bash scripts/run-graphics.sh [out.png]
#
# For an interactive window instead, drop `-display none` and the QMP capture:
#   qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M -device ramfb \
#     -kernel target/aarch64-unknown-none/debug/mansa-kernel
set -uo pipefail

OUT="${1:-splash.png}"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -device ramfb -display none -serial file:serial.log \
    -qmp tcp:127.0.0.1:4444,server,nowait -kernel "$BIN" &
QEMU_PID=$!

# Wait until boot + the userspace demos finish (shell starting = end of boot).
for _ in $(seq 1 14); do
    grep -q "interactive shell" serial.log 2>/dev/null && break
    sleep 1
done
sleep 1

python3 scripts/qmp-screendump.py 127.0.0.1:4444 "$OUT"
RC=$?
kill "$QEMU_PID" 2>/dev/null
wait "$QEMU_PID" 2>/dev/null
echo "Wrote $OUT (rc=$RC)"
exit $RC
