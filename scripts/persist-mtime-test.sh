#!/usr/bin/env bash
# Prove file MTIMES survive a reboot: boot 1 writes a file (RTC-stamped) and syncs
# it to a virtio-blk disk; boot 2 (same image) restores it from disk and `stat`
# must still show a real last-modified date (a 20xx date, NOT "unknown") -- proving
# the mtime was persisted alongside the content. Needs qemu + python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
IMG="${TMPDIR:-/tmp}/afrisovera-persist-mtime.img"
rm -f "$IMG"
head -c 4194304 /dev/zero > "$IMG" # fresh 4 MiB blank disk

boot() { # $1 = serial/qmp base port, rest = shell commands
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
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}"     python3 scripts/shell-demo.py "$sp" "$((sp+1))" "${TMPDIR:-/tmp}/persist-mtime.png" "$@" 2>&1
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
}

echo "[boot 1] writing persistmt.txt, syncing, and stat-ing..."
B1="$(boot 4534 "write persistmt.txt hi" "sync" "stat persistmt.txt")"
echo "$B1" | grep -q "synced" || { echo "PERSIST-MTIME TEST FAILED (sync did not run in boot 1)"; exit 1; }

echo "[boot 2] fresh boot on the same disk -- stat persistmt.txt (mtime from disk)..."
B2="$(boot 4536 "stat persistmt.txt")"
echo "$B2" | grep -aiE "stat persistmt|modified" | sed 's/^/  /'

python3 - <<PY
import re, sys
b2 = """$B2"""
m = re.search(r"stat persistmt\.txt: \d+ bytes, modified (20\d\d)-\d\d-\d\d", b2)
if not m:
    if "modified unknown" in b2:
        print("PERSIST-MTIME TEST FAILED (mtime came back as 'unknown' -- not persisted)")
    else:
        print("PERSIST-MTIME TEST FAILED (no stat line with a persisted date in boot 2)")
    sys.exit(1)
year = int(m.group(1))
if not (2025 <= year <= 2100):
    print(f"PERSIST-MTIME TEST FAILED (mtime year {year} out of range)"); sys.exit(1)
print(f"PERSIST-MTIME TEST PASSED  (persistmt.txt mtime {m.group(1)}-... survived reboot via disk)")
PY
