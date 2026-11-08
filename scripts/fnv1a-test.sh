#!/usr/bin/env bash
# Verify the `fnv1a` command computes the 32-bit FNV-1a hash. Canonical published
# FNV-1a 32-bit test vectors:
#   "a"      -> 0xe40c292c
#   "foobar" -> 0xbf9cf968
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4734,server,nowait \
    -qmp tcp:127.0.0.1:4735,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/fnv1a.out"
python3 scripts/shell-demo.py 4734 4735 "${TMPDIR:-/tmp}/fnv1a.png" "fnv1a a" "fnv1a foobar" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "fnv1a: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["fnv1a: 0xe40c292c", "fnv1a: 0xbf9cf968"]
missing = [w for w in want if w not in s]
if missing:
    print(f"FNV1A TEST FAILED (missing: {missing})"); sys.exit(1)
print("FNV1A TEST PASSED  (a -> 0xe40c292c; foobar -> 0xbf9cf968)")
PY
