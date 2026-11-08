#!/usr/bin/env bash
# Verify the `cmp` command compares two VFS files byte-by-byte: a file vs itself is
# identical (about.txt = 128 bytes); motd vs about.txt differ at byte 0 (0x57 'W' vs
# 0x41 'A'); a missing file is reported. Deterministic on the seeded files.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4962,server,nowait \
    -qmp tcp:127.0.0.1:4963,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/cmp.out"
python3 scripts/shell-demo.py 4962 4963 "${TMPDIR:-/tmp}/cmp.png" \
    "cmp about.txt about.txt" "cmp motd about.txt" "cmp about.txt nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cmp:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "cmp: about.txt about.txt are identical (128 bytes)",
    "cmp: motd about.txt differ at byte 0 (0x57 vs 0x41)",
    "cmp: nope.txt not found",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"CMP TEST FAILED (missing: {missing})"); sys.exit(1)
print("CMP TEST PASSED  (self=identical 128B; motd/about.txt differ at byte 0 W/A; missing reported)")
PY
