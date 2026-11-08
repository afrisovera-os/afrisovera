#!/usr/bin/env bash
# Verify the `wordn` command prints the Nth (1-based) whitespace-separated word of
# a VFS file, counting ACROSS all lines -- distinct from cut (a field of each line)
# and line (a whole line). Uses the seeded two-line about.txt: word 1 = "AfriSovera:",
# word 2 = "one", word 9 = "AI" (the first word of line 2, proving it spans line
# breaks). Also checks the out-of-range, no-such-file, and usage paths.
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
OUT="${TMPDIR:-/tmp}/wordn.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordn.png" \
    "wordn 1 about.txt" "wordn 2 about.txt" "wordn 9 about.txt" \
    "wordn 100 about.txt" "wordn 1 nope.txt" "wordn" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordn:|usage: wordn" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "wordn: AfriSovera:",                       # word 1
    "wordn: one",                               # word 2
    "wordn: AI",                                # word 9 -- first word of line 2 (spans lines)
    "wordn: file has fewer than 100 words",     # out of range
    "wordn: no such file: nope.txt",            # missing file
    "usage: wordn <n> <file>",                  # missing args
]
for want in checks:
    if want not in s:
        print(f"WORDN TEST FAILED (missing '{want}')"); sys.exit(1)

print("WORDN TEST PASSED  (word 1/2 in line 1, word 9 = 'AI' across the line break; out-of-range + no-such-file + usage)")
PY
