#!/usr/bin/env bash
# Prove real file persistence across reboots: boot 1 writes a file and `sync`s it to
# a virtio-blk disk; boot 2 (same disk image) must restore it and `cat` it back.
# Needs qemu + python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
IMG="${TMPDIR:-/tmp}/afrisovera-persist.img"
rm -f "$IMG"
head -c 4194304 /dev/zero > "$IMG" # fresh 4 MiB blank disk

boot() { # $1 = qmp/serial base port offset, rest = shell commands
    local sp=$1; shift
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none \
        -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" \
        -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" -kernel "$BIN" &
    local qpid=$!
    # 2.908: no fixed window. shell-demo.py's connect_serial() retries until QEMU's
    # serial socket listens (measured 0.53/0.56/1.14s) and wait_ready() returns on
    # the PROMPT, so the 5s guess bought nothing on the happy path. BOOT_GRACE goes
    # 6 -> 10 so the SILENT-guest pre-typing floor is UNCHANGED at 14s from QEMU
    # start (was 5 sleep + 3 quiet + 6 grace; now ~1 attach + 3 quiet + 10 grace).
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}"     python3 scripts/shell-demo.py "$sp" "$((sp+1))" "${TMPDIR:-/tmp}/persist.png" "$@" 2>&1
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
}

echo "[boot 1] writing notes.txt and syncing to disk..."
B1="$(boot 4530 "write notes.txt HELLO-PERSISTENT" "sync")"
echo "$B1" | grep -q "synced" || { echo "PERSIST TEST FAILED (sync did not run)"; exit 1; }

echo "[boot 2] fresh boot on the same disk — reading notes.txt back..."
B2="$(boot 4532 "cat notes.txt")"
echo "$B2" | grep -i "HELLO-PERSISTENT" || true

# The definitive proof: a file written+synced in boot 1 is readable in boot 2 (a
# fresh QEMU process) with no re-write — so it came from the disk.
echo "$B2" | grep -q "HELLO-PERSISTENT" \
    && echo "PERSIST TEST PASSED  (notes.txt written in boot 1 survived into boot 2 via disk)" \
    || { echo "PERSIST TEST FAILED (file contents not restored)"; exit 1; }
