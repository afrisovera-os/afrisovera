#!/usr/bin/env bash
# Build the Raspberry Pi 4 (BCM2711) variant as a flat binary the VideoCore
# firmware could load as `kernel8.img` -- docs/15-hardware-bringup.md
# "Suggested order" item 1's flat-binary half.
#
# This is a COMPILE-LEVEL check only: no QEMU machine for a bare BCM2711 has
# been established in this tree (board.rs still calls board-rpi4 "map only"),
# and there is no physical RPi4 available to this session, so this script does
# NOT attempt to boot the image anywhere. It builds, objcopies, and asserts the
# two properties a flat binary must have before that question is even
# reachable: the ELF's entry point is the fixed address the firmware loads
# `kernel8.img` at (0x8_0000 -- see kernel/linker-rpi4.ld), and objcopy
# actually produces a non-empty binary from it.
set -uo pipefail

cargo build -p mansa-kernel --no-default-features --features board-rpi4 \
    || { echo "RPI4 IMAGE FAILED (build: board-rpi4 variant does not compile)"; exit 1; }

ELF=target/aarch64-unknown-none/debug/mansa-kernel
OD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objdump"
ENTRY="$("$OD" -f "$ELF" | sed -n 's/^start address: //p')"
[ "$ENTRY" = "0x0000000000080000" ] \
    || { echo "RPI4 IMAGE FAILED (entry point $ENTRY != 0x80000)"; exit 1; }

OC="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
IMG=target/mansa-rpi4.img
rm -f "$IMG"          # never report a previous run's image if objcopy fails
"$OC" -O binary "$ELF" "$IMG" \
    || { echo "RPI4 IMAGE FAILED (objcopy)"; exit 1; }

SIZE="$(wc -c < "$IMG")"
[ "$SIZE" -gt 0 ] \
    || { echo "RPI4 IMAGE FAILED (objcopy produced an empty file)"; exit 1; }

echo "RPI4 IMAGE OK: entry=$ENTRY size=${SIZE} bytes -> $IMG"
echo "NOTE: compile-level check only -- not booted on QEMU or real hardware."
