#!/usr/bin/env bash
# Verify the `caesar` command shifts ASCII letters by N (rot-N), case-preserving,
# non-letters passing through. Deterministic, hand-checkable:
#   caesar 3 HELLO       -> "caesar: KHOOR"
#   caesar 13 AfriSovera -> "caesar: NsevFbiren"   (rot13, case preserved)
#   caesar 1 xyz         -> "caesar: yza"           (wrap z->a)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4712,server,nowait \
    -qmp tcp:127.0.0.1:4713,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/caesar.out"
python3 scripts/shell-demo.py 4712 4713 "${TMPDIR:-/tmp}/caesar.png" "caesar 3 HELLO" "caesar 13 AfriSovera" "caesar 1 xyz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "caesar: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["caesar: KHOOR", "caesar: NsevFbiren", "caesar: yza"]
missing = [w for w in want if w not in s]
if missing:
    print(f"CAESAR TEST FAILED (missing exact substring(s): {missing})"); sys.exit(1)
print("CAESAR TEST PASSED  (HELLO->KHOOR; rot13 AfriSovera->NsevFbiren; xyz->yza wrap)")
PY
