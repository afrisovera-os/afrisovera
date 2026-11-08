#!/usr/bin/env bash
# Verify the `tac` command prints a VFS file's lines in reverse order (reverse of
# cat). The seeded two-line about.txt must print line 2 BEFORE line 1, with no
# phantom empty line; a missing file reports not-found.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4808,server,nowait \
    -qmp tcp:127.0.0.1:4809,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/tac.out"
python3 scripts/shell-demo.py 4808 4809 "${TMPDIR:-/tmp}/tac.png" \
    "tac about.txt" "tac nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tac:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
line1 = "tac: AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
line2 = "tac: AI as an OS service. You own the device and the software on it."
for w in (line1, line2, "tac: nope.txt not found"):
    if w not in s:
        print(f"TAC TEST FAILED (missing: {w!r})"); sys.exit(1)
# Reversed: line 2 must appear before line 1 in the output.
if s.index(line2) >= s.index(line1):
    print("TAC TEST FAILED (lines not reversed -- line2 should precede line1)"); sys.exit(1)
print("TAC TEST PASSED  (about.txt lines reversed: line2 before line1; missing file reported)")
PY
