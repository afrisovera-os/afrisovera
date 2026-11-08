#!/usr/bin/env bash
# Verify the `nl` command numbers the lines of a VFS file (like `cat -n`). Uses the
# seeded two-line file about.txt (one embedded '\n', no trailing newline), so the
# output is deterministic: line 1 and line 2 numbered, no phantom blank third line.
# Also checks the missing-file path.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4754,server,nowait \
    -qmp tcp:127.0.0.1:4755,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/nl.out"
python3 scripts/shell-demo.py 4754 4755 "${TMPDIR:-/tmp}/nl.png" \
    "cat about.txt" "nl about.txt" "nl nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nl:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "nl:    1  AfriSovera: one codebase, WASM-native apps, no gatekeeper store,",
    "nl:    2  AI as an OS service. You own the device and the software on it.",
    "nl: nope.txt not found",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"NL TEST FAILED (missing: {missing})"); sys.exit(1)
# No phantom line 3 from a trailing-newline-free file.
if "nl:    3" in s:
    print("NL TEST FAILED (unexpected line 3 -- phantom empty line not skipped)"); sys.exit(1)
print("NL TEST PASSED  (about.txt -> lines 1 & 2 numbered, no phantom line 3; missing file reported)")
PY
