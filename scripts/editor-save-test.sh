#!/usr/bin/env bash
# Verify the editor persists typed text to the VFS: start the `edit` EL0 app,
# type HELLO via raw keystrokes, quit with '~', and confirm the kernel saved the
# text to edited.txt (and that `cat` reads it back). Proves a real type -> persist
# path through the EL0 app + input routing + filesystem. RAM-only.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4638,server,nowait \
    -qmp tcp:127.0.0.1:4639,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/editor-save.out"
# edit -> type HELLO (raw) -> '~' quits (raw 0x7e) -> cat the saved file
python3 scripts/shell-demo.py 4638 4639 "${TMPDIR:-/tmp}/editor-save.png" \
    "edit" ":HELLO" ":\x7e" "cat edited.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "edit\]|saved|edited.txt|HELLO" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
if not re.search(r"\[edit\] saved 5 bytes to edited\.txt", s):
    print("EDITOR-SAVE TEST FAILED (no save confirmation for 5 bytes)"); sys.exit(1)
# the cat output must show HELLO on its own (after the command echo)
if "HELLO" not in s.split("cat edited.txt", 1)[-1]:
    print("EDITOR-SAVE TEST FAILED (cat did not read back HELLO)"); sys.exit(1)
print("EDITOR-SAVE TEST PASSED  (typed HELLO -> saved 5 bytes to edited.txt -> cat read it back)")
PY
