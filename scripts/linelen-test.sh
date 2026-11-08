#!/usr/bin/env bash
# Verify the `linelen <n> <file>` command reports the byte length of the Nth
# (1-based) line of a VFS file (newline excluded). DISTINCT from `line` (prints
# the CONTENT of a line), `avglen` (AVERAGE line length), and `wc` (whole-file
# totals). Self-checking: python recomputes the expected lengths from the KNOWN
# seed constants (about.txt's two lines and motd) and asserts them, plus the
# fewer-than / no-such-file / usage paths.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/linelen.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/linelen.png" \
    "linelen 1 about.txt" "linelen 2 about.txt" "linelen 1 motd" \
    "linelen 3 about.txt" "linelen 1 nope.txt" "linelen" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "linelen:|usage: linelen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constants (must match kernel/src/vfs.rs).
about_l1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
about_l2 = "AI as an OS service. You own the device and the software on it."
motd     = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

# about.txt's two lines sum to 127 bytes (matches the `avglen` suite).
assert len(about_l1) + len(about_l2) == 127, (len(about_l1), len(about_l2))

checks = [
    f"linelen: line 1 is {len(about_l1)} bytes",   # 64
    f"linelen: line 2 is {len(about_l2)} bytes",   # 63
    f"linelen: line 1 is {len(motd)} bytes",       # motd, 63
    "linelen: file has fewer than 3 lines",
    "linelen: no such file: nope.txt",
    "usage: linelen <n> <file>",
]
for want in checks:
    if want not in s:
        print(f"LINELEN TEST FAILED (missing '{want}')"); sys.exit(1)

print("LINELEN TEST PASSED  (line lengths exact vs seed constants, fewer-than/no-file/usage)")
PY
