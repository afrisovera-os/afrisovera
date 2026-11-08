#!/usr/bin/env bash
# virtio-blk check: boot with a raw disk image attached (modern transport), run the
# `disk` command, and confirm the driver can write a sector and read it back.
# Needs qemu + python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
IMG="${TMPDIR:-/tmp}/afrisovera-disk.img"
[ -f "$IMG" ] || head -c 4194304 /dev/zero > "$IMG" # 4 MiB blank disk

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
    -serial tcp:127.0.0.1:4520,server,nowait \
    -qmp tcp:127.0.0.1:4521,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4520 4521 "${TMPDIR:-/tmp}/disk-test.png" "disk" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
echo "$SER" | grep -iE "virtio-blk|sectors" || true
echo "$SER" | grep -q "blk self-test OK" \
    && echo "DISK TEST PASSED  (virtio-blk wrote and read back a sector)" \
    || { echo "DISK TEST FAILED"; exit 1; }
