#!/usr/bin/env bash
# Prove capability-gated randomness: the `random` shell command runs a WASM app
# twice - once with CAP_RANDOM granted (it draws two values via the host random()
# function) and once denied (the kernel refuses the host call). Asserts both the
# granted draw and the denial. RAM-only (no -netdev/-drive). Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4602,server,nowait \
    -qmp tcp:127.0.0.1:4603,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4602 4603 "${TMPDIR:-/tmp}/rand-cap.png" "random" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

echo "$SER" | grep -iE "CAP_RANDOM|wasm app|DENIED|capability|TRAP" | sed 's/^/  /'

echo "$SER" | grep -q "CAP_RANDOM GRANTED" \
    || { echo "RAND-CAP TEST FAILED (granted run did not execute)"; exit 1; }
echo "$SER" | grep -q "app called host 'random' without the capability" \
    || { echo "RAND-CAP TEST FAILED (un-capable app was NOT denied)"; exit 1; }
echo "RAND-CAP TEST PASSED  (CAP_RANDOM granted app drew values; un-capable app was denied)"
