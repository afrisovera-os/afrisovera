#!/usr/bin/env bash
# Verify the `demorse <morse>` command DECODES Morse code back to text -- the
# inverse of `morse`. It inverts morse's exact format: letter-codes separated by
# single spaces, words by " / ". Case is NOT preserved (morse uppercases input),
# so letters decode to lowercase; unknown codes -> '?'. Self-checking: python
# builds the SAME morse table, ENCODES known strings to morse, feeds them to
# demorse, and asserts demorse(morse(x)) == x (round-trip).
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
OUT="${TMPDIR:-/tmp}/demorse.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/demorse.png" \
    "demorse ... --- ..." "demorse .... . .-.. .-.. ---" \
    "demorse .... .. / - .... . .-. ." "demorse ..--.. xyz" "demorse" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "demorse:|usage: demorse" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The SAME morse table as kernel/src/shell.rs (letters + digits).
TABLE = {
    'a': '.-', 'b': '-...', 'c': '-.-.', 'd': '-..', 'e': '.', 'f': '..-.',
    'g': '--.', 'h': '....', 'i': '..', 'j': '.---', 'k': '-.-', 'l': '.-..',
    'm': '--', 'n': '-.', 'o': '---', 'p': '.--.', 'q': '--.-', 'r': '.-.',
    's': '...', 't': '-', 'u': '..-', 'v': '...-', 'w': '.--', 'x': '-..-',
    'y': '-.--', 'z': '--..',
    '0': '-----', '1': '.----', '2': '..---', '3': '...--', '4': '....-',
    '5': '.....', '6': '-....', '7': '--...', '8': '---..', '9': '----.',
}

def morse(text):  # mirror the kernel's morse encoder (word gap " / ")
    words = text.split(' ')
    return ' / '.join(' '.join(TABLE[c] for c in w if c in TABLE) for w in words)

# Round-trip: demorse(morse(x)) == x for lowercase alnum inputs.
for original in ["sos", "hello", "hi there"]:
    code = morse(original)
    want = f"demorse: {original}"
    if want not in s:
        print(f"DEMORSE TEST FAILED (round-trip for '{original}' via '{code}': missing '{want}')")
        sys.exit(1)

# Unknown codes -> '?': "..--.. xyz" are both not in the letter/digit table.
if "demorse: ??" not in s:
    print("DEMORSE TEST FAILED (unknown codes should decode to '??')"); sys.exit(1)
if "usage: demorse <morse>" not in s:
    print("DEMORSE TEST FAILED (missing usage line)"); sys.exit(1)

print("DEMORSE TEST PASSED  (decode inverts morse: demorse(morse(x))==x; word gaps, unknown->'?', empty->usage)")
PY
