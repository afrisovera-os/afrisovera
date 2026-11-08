#!/usr/bin/env bash
# Verify `disemvowelf <file>` removes ALL ASCII vowels (aeiou, both cases) from a whole VFS file,
# keeping consonants/digits/punctuation/spaces/line structure -- the file analogue of disemvowel.
# Tests motd (single line) and about.txt (two lines, checked line-by-line due to serial CRLF), plus
# a missing file. Self-checking: python strips vowels.
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
OUT="${TMPDIR:-/tmp}/disemvowelf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/disemvowelf.png" \
    "echo warmup" "disemvowelf motd" "disemvowelf about.txt" "disemvowelf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "disemvowelf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT_L1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
ABOUT_L2 = "AI as an OS service. You own the device and the software on it."

def strip_vowels(t):                 # DERIVED reference
    return "".join(c for c in t if c.lower() not in "aeiou")

want_motd = "disemvowelf: " + strip_vowels(MOTD)
if want_motd not in s:
    print(f"DISEMVOWELF TEST FAILED (missing motd: {want_motd!r})"); sys.exit(1)

# about.txt prints on two lines; serial inserts \r, so check each line as a substring
for ln in (ABOUT_L1, ABOUT_L2):
    stripped = strip_vowels(ln)
    if stripped not in s:
        print(f"DISEMVOWELF TEST FAILED (missing about line: {stripped!r})"); sys.exit(1)

assert "disemvowelf: no such file: nope.txt" in s
print("DISEMVOWELF TEST PASSED  (motd + about.txt both lines vs python vowel-strip; nope.txt errors)")
PY
