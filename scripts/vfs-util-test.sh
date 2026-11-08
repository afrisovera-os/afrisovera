#!/usr/bin/env bash
# Verify the VFS file utilities: write a file, `cp` it, `mv` the copy, and confirm
# the destination holds the content while the moved-from file is gone. RAM-only.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4640,server,nowait \
    -qmp tcp:127.0.0.1:4641,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/vfs-util.out"
python3 scripts/shell-demo.py 4640 4641 "${TMPDIR:-/tmp}/vfs-util.png" \
    "write greet HELLOAFRI" "cp greet copy1" "cat copy1" "mv copy1 copy2" "cat copy2" "cat copy1" \
    > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "copied|moved|HELLOAFRI|no such file" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
# The two `cat`s of the destination (copy1 after cp, copy2 after mv) must show the content.
if s.count("HELLOAFRI") < 3:  # write echo is not a cat; expect greet-write + 2 successful cats
    pass
ok_copy = "copied greet -> copy1" in s
ok_move = "moved copy1 -> copy2" in s
# copy2 must contain the content; copy1 must be gone (cat copy1 -> not found) after the mv
after_mv = s.split("moved copy1 -> copy2", 1)[-1]
ok_dest = "HELLOAFRI" in after_mv
ok_gone = "no such file: copy1" in after_mv
if ok_copy and ok_move and ok_dest and ok_gone:
    print("VFS-UTIL TEST PASSED  (cp copied, mv moved, source removed, content preserved)")
    sys.exit(0)
print(f"VFS-UTIL TEST FAILED (cp={ok_copy} mv={ok_move} dest={ok_dest} src_gone={ok_gone})")
sys.exit(1)
PY
