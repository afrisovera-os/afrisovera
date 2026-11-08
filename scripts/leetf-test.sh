#!/usr/bin/env bash
# Verify `leetf <file>` leetspeaks a whole VFS file with the same a4/e3/i1/o0/s5/t7 substitution as
# `leet` (case-insensitive match, other bytes passthrough) -- the file analogue of leet. Tests motd
# and about.txt (two lines, checked line-by-line due to serial CRLF), plus a missing file.
# Self-checking: python applies the same mapping.
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
OUT="${TMPDIR:-/tmp}/leetf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/leetf.png" \
    "echo warmup" "leetf motd" "leetf about.txt" "leetf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "leetf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT_L1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
ABOUT_L2 = "AI as an OS service. You own the device and the software on it."

MAP = {"a":"4","e":"3","i":"1","o":"0","s":"5","t":"7"}
def leet(t):                         # DERIVED reference: fold case for match, keep original byte
    return "".join(MAP.get(c.lower(), c) for c in t)

want_motd = "leetf: " + leet(MOTD)
if want_motd not in s:
    print(f"LEETF TEST FAILED (missing motd: {want_motd!r})"); sys.exit(1)

for ln in (ABOUT_L1, ABOUT_L2):      # two-line file: check each line as a substring
    if leet(ln) not in s:
        print(f"LEETF TEST FAILED (missing about line: {leet(ln)!r})"); sys.exit(1)

assert "leetf: no such file: nope.txt" in s
print("LEETF TEST PASSED  (motd + about.txt both lines vs python a4e3i1o0s5t7; nope.txt errors)")
PY
