#!/usr/bin/env bash
# Verify the `maxline <file>` command reports which line is the LONGEST in a VFS
# file (its 1-based line number + byte length), ties resolving to the FIRST. The
# line-level analog of `longestword`; DISTINCT from `linelen` (a specific line's
# length), `avglen` (average), and `wc` (whole-file totals). Self-checking:
# python recomputes the longest line (first-on-tie) from the KNOWN seed constants
# and asserts the exact output.
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
OUT="${TMPDIR:-/tmp}/maxline.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/maxline.png" \
    "maxline about.txt" "maxline motd" "maxline nope.txt" "maxline" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "maxline:|usage: maxline" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constants (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
motd  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def maxline(text):
    best_no, best_len = 0, 0
    for i, l in enumerate(text.split("\n"), start=1):
        if best_no == 0 or len(l) > best_len:   # strict > keeps FIRST on tie
            best_no, best_len = i, len(l)
    return best_no, best_len

for name, text in (("about.txt", about), ("motd", motd)):
    no, ln = maxline(text)
    want = f"maxline: line {no} is longest ({ln} bytes)"
    if want not in s:
        print(f"MAXLINE TEST FAILED ({name}: missing '{want}')"); sys.exit(1)

# about.txt line 1 (64) beats line 2 (63); motd's single line is 63.
assert maxline(about) == (1, 64), maxline(about)
assert maxline(motd) == (1, 63), maxline(motd)

if "maxline: no such file: nope.txt" not in s:
    print("MAXLINE TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: maxline <file>" not in s:
    print("MAXLINE TEST FAILED (missing usage line)"); sys.exit(1)

print("MAXLINE TEST PASSED  (longest line + length exact vs seed constants, no-file/usage)")
PY
