#!/usr/bin/env bash
# Verify the `vowelcount <file>` command counts the ASCII vowels (a,e,i,o,u both
# cases) and the total letters in a VFS file. DISTINCT from `vowels` (which counts
# INLINE TEXT typed as an argument), `charcount` (one specific char), and
# `histogram` (byte frequency). Self-checking: python re-derives vowels/letters
# from the seed strings (about.txt -> 46/100, motd -> 23/50).
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
OUT="${TMPDIR:-/tmp}/vowelcount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/vowelcount.png" \
    "echo warmup" "vowelcount about.txt" "vowelcount motd" \
    "vowelcount nofile.txt" "vowelcount" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vowelcount:|usage: vowelcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def line(name, text):
    letters = sum(c.isascii() and c.isalpha() for c in text)
    vowels = sum(c.isalpha() and c.lower() in "aeiou" for c in text)
    return f"vowelcount: {name} = {vowels} vowels of {letters} letters"

for name, text in [("about.txt", ABOUT), ("motd", MOTD)]:
    want = line(name, text)
    if want not in s:
        print(f"VOWELCOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the headline numbers
assert line("about.txt", ABOUT) == "vowelcount: about.txt = 46 vowels of 100 letters"
assert line("motd", MOTD) == "vowelcount: motd = 23 vowels of 50 letters"

if "vowelcount: no such file: nofile.txt" not in s:
    print("VOWELCOUNT TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: vowelcount <file>" not in s:
    print("VOWELCOUNT TEST FAILED (missing usage line)"); sys.exit(1)

print("VOWELCOUNT TEST PASSED  (about.txt 46/100, motd 23/50 re-derived vs python; "
      "not-found + usage)")
PY
