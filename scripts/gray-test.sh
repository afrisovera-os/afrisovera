#!/usr/bin/env bash
# Verify the `gray` command computes the reflected binary Gray code (n ^ n>>1)
# and decodes it back (round-trip). Deterministic:
#   gray 4   -> 4^2  = 6   = 0x6,  decoded 4
#   gray 7   -> 7^3  = 4   = 0x4,  decoded 7
#   gray 255 -> 255^127=128= 0x80, decoded 255
# The "decoded N == N" on each case proves the prefix-xor inverse works.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4752,server,nowait \
    -qmp tcp:127.0.0.1:4753,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/gray.out"
python3 scripts/shell-demo.py 4752 4753 "${TMPDIR:-/tmp}/gray.png" "gray 4" "gray 7" "gray 255" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "gray: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
cases = [
    ("gray: 4 -> gray 0x6", "decoded 4"),
    ("gray: 7 -> gray 0x4", "decoded 7"),
    ("gray: 255 -> gray 0x80", "decoded 255"),
]
for enc, dec in cases:
    line = next((l for l in s.splitlines() if l.startswith(enc)), "")
    if not line or dec not in line:
        print(f"GRAY TEST FAILED (expected a line '{enc} ... {dec}'; got {line!r})"); sys.exit(1)
print("GRAY TEST PASSED  (4->0x6, 7->0x4, 255->0x80; all round-trip decoded)")
PY
