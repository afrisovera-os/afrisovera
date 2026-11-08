#!/usr/bin/env bash
# Verify the `line` command prints the Nth (1-based) line of a VFS file -- distinct
# from head/tail (byte ranges), cut (a field of each line), and nl (numbers all).
# Uses the seeded two-line about.txt: line 1 and line 2 are distinct file lines
# (globally unambiguous substrings), an out-of-range index reports the line count,
# plus the no-such-file and usage paths.
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
OUT="${TMPDIR:-/tmp}/line.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/line.png" \
    "line 1 about.txt" "line 2 about.txt" "line 3 about.txt" \
    "line 1 nope.txt" "line" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "AfriSovera: one|AI as an OS|fewer than|no such file|usage: line" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,",  # line 1
    "AI as an OS service. You own the device and the software on it.",   # line 2
    "line: file has fewer than 3 lines",                                 # out of range
    "line: no such file: nope.txt",                                      # missing file
    "usage: line <n> <file>",                                            # missing args
]
for want in checks:
    if want not in s:
        print(f"LINE TEST FAILED (missing '{want}')"); sys.exit(1)

print("LINE TEST PASSED  (line 1/2 print the right file lines, out-of-range + no-such-file + usage handled)")
PY
