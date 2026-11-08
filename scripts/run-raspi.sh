#!/usr/bin/env bash
# Build the Raspberry Pi (BCM2837) variant and boot it on QEMU's raspi3b machine.
# This is a minimal UART bring-up (MMU off; GIC/timer/framebuffer are virt-only) —
# proof AfriSovera targets a second, real-hardware-shaped board. The PL011 (UART0
# @ 0x3F20_1000) is QEMU raspi3b's serial0.
set -uo pipefail

cargo build -p mansa-kernel --no-default-features --features board-raspi
OC="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
OUT=target/mansa-raspi.img
"$OC" -O binary target/aarch64-unknown-none/debug/mansa-kernel "$OUT"
echo "wrote $OUT ($(wc -c < "$OUT") bytes); booting QEMU raspi3b (Ctrl-A X to quit)..."
qemu-system-aarch64 -M raspi3b -kernel "$OUT" -serial stdio -display none
