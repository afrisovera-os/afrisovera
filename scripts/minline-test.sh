#!/usr/bin/env bash
# Verify the `minline <file>` command reports which line is the SHORTEST in a VFS
# file (its 1-based line number + byte length), ties resolving to the FIRST. The
# min-analog of `maxline`; DISTINCT from `linelen` (a specific line's length),
# `avglen` (average), and `wc` (whole-file totals). Self-checking: python
# recomputes the shortest line (first-on-tie) from the KNOWN seed constants and
# asserts the exact output. Note about.txt's shortest is line 2 (63) -- the
# OPPOSITE of maxline's line 1 (64).
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
OUT="${TMPDIR:-/tmp}/minline.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/minline.png" \
    "minline about.txt" "minline motd" "minline nope.txt" "minline" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "minline:|usage: minline" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constants (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
motd  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def minline(text):
    best_no, best_len = 0, 0
    for i, l in enumerate(text.split("\n"), start=1):
        if best_no == 0 or len(l) < best_len:   # strict < keeps FIRST on tie
            best_no, best_len = i, len(l)
    return best_no, best_len

for name, text in (("about.txt", about), ("motd", motd)):
    no, ln = minline(text)
    want = f"minline: line {no} is shortest ({ln} bytes)"
    if want not in s:
        print(f"MINLINE TEST FAILED ({name}: missing '{want}')"); sys.exit(1)

# about.txt line 2 (63) is shorter than line 1 (64); motd's single line is 63.
assert minline(about) == (2, 63), minline(about)
assert minline(motd) == (1, 63), minline(motd)

if "minline: no such file: nope.txt" not in s:
    print("MINLINE TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: minline <file>" not in s:
    print("MINLINE TEST FAILED (missing usage line)"); sys.exit(1)

print("MINLINE TEST PASSED  (shortest line + length exact vs seed constants, no-file/usage)")
PY
