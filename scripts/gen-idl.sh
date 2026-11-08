#!/usr/bin/env bash
# Regenerate Rust from the AfriIDL interface definitions. The AfriIDL compiler is
# a host tool, so we build it for the host triple (not the kernel's bare-metal
# target).
set -euo pipefail

HOST="$(rustc -vV | sed -n 's/^host: //p')"
mkdir -p generated
cargo run -q -p afriidl --target "$HOST" -- idl/ipc.afriidl -o generated/ipc.rs
echo "Regenerated: generated/ipc.rs"
