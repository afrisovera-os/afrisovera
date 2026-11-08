#!/usr/bin/env bash
# Build the kernel as a raw arm64 `Image` (in addition to the ELF). Booting this
# way — via QEMU's raw -kernel path, U-Boot, or real firmware — hands the kernel a
# LIVE device tree pointer in x0 (the ELF path gets x0=0). The kernel carries an
# arm64 Linux Image header and is linked at 0x4020_0000 (where QEMU virt loads it).
set -euo pipefail

cargo build -p mansa-kernel
OC="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
ELF=target/aarch64-unknown-none/debug/mansa-kernel
OUT=target/mansa-kernel.img
"$OC" -O binary "$ELF" "$OUT"
echo "wrote $OUT ($(wc -c < "$OUT") bytes)"
