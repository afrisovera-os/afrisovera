#!/usr/bin/env bash
# Build the compiled-Rust EL0 userland app and emit its raw binary
# (kernel/src/rustapp.bin, which the kernel include_bytes!'s and spawns at EL0).
# Re-run this after changing userland/rustapp.
set -euo pipefail

OC="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
[ -x "$OC" ] || OC="$OC.exe"
for app in rustapp niavm; do
    cargo build -p "$app"
    "$OC" -O binary "target/aarch64-unknown-none/debug/$app" "kernel/src/$app.bin"
    echo "wrote kernel/src/$app.bin ($(wc -c < "kernel/src/$app.bin") bytes)"
done

# The PC (x86-64): the same rustapp source for the PC kernel's ring 3, linked at its
# USER_BASE (userland/rustapp/linker-x86_64.ld). Position-independent code, with the GOT
# filled in at link time (--apply-dynamic-relocs) since it only ever loads at that address.
# Needs: rustup target add x86_64-unknown-none --toolchain 1.97.1
for app in rustapp niavm; do
    RUSTFLAGS="-C link-arg=--apply-dynamic-relocs" cargo build -p "$app" --target x86_64-unknown-none
    "$OC" -O binary "target/x86_64-unknown-none/debug/$app" "kernel/src/pc/$app-x86_64.bin"
    echo "wrote kernel/src/pc/$app-x86_64.bin ($(wc -c < "kernel/src/pc/$app-x86_64.bin") bytes)"
done
