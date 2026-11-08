#!/usr/bin/env bash
# Boot the Mansa kernel in QEMU (AArch64 `virt` board).
#
# Requires: qemu-system-aarch64 on PATH.
#   - Windows: install from https://www.qemu.org/download/#windows and add the
#     install dir (e.g. C:\Program Files\qemu) to PATH.
#   - Debian/Ubuntu: sudo apt install qemu-system-arm
#   - macOS:  brew install qemu
#
# Ctrl-A then X quits QEMU (-nographic serial mode).
set -euo pipefail

PROFILE="${1:-debug}"
BIN="target/aarch64-unknown-none/${PROFILE}/mansa-kernel"

if [[ "${PROFILE}" == "release" ]]; then
    cargo build -p mansa-kernel --release
else
    cargo build -p mansa-kernel
fi

exec qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a72 \
    -smp 1 \
    -m 256M \
    -nographic \
    -kernel "${BIN}"
