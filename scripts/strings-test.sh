#!/usr/bin/env bash
# Verify the `strings` command extracts maximal printable-ASCII runs (min length 4)
# from a VFS file. The seeded two-line about.txt (one embedded '\n') yields exactly
# two runs -- one per line; a missing file reports not-found.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4784,server,nowait \
    -qmp tcp:127.0.0.1:4785,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/strings.out"
python3 scripts/shell-demo.py 4784 4785 "${TMPDIR:-/tmp}/strings.png" \
    "strings about.txt" "strings nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "strings:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "strings: AfriSovera: one codebase, WASM-native apps, no gatekeeper store,",
    "strings: AI as an OS service. You own the device and the software on it.",
    "strings: nope.txt not found",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"STRINGS TEST FAILED (missing: {missing})"); sys.exit(1)
print("STRINGS TEST PASSED  (about.txt -> two printable runs; missing file reported)")
PY
