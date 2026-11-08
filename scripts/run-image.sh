#!/usr/bin/env bash
# Boot the raw arm64 Image so QEMU passes a LIVE device tree in x0. Watch the boot
# log for "device tree (live)" with a RAM size matching -m (proof of real hardware
# discovery). Usage: run-image.sh [MEM]   e.g. run-image.sh 512M
set -uo pipefail

MEM="${1:-512M}"
bash scripts/build-image.sh
echo "booting raw Image with -m $MEM (Ctrl-A X to quit)..."
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m "$MEM" -nographic \
    -kernel target/mansa-kernel.img
