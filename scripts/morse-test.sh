#!/usr/bin/env bash
# Verify the `morse` command encodes text to International Morse code: single space
# between letters, " / " between words. SOS -> ... --- ...; HELLO -> .... . .-.. .-.. ---;
# "SOS SOS" -> the two words joined by a slash word-gap.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4772,server,nowait \
    -qmp tcp:127.0.0.1:4773,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/morse.out"
python3 scripts/shell-demo.py 4772 4773 "${TMPDIR:-/tmp}/morse.png" \
    "morse SOS" "morse HELLO" "morse SOS SOS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "morse:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "morse: ... --- ...",
    "morse: .... . .-.. .-.. ---",
    "morse: ... --- ... / ... --- ...",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"MORSE TEST FAILED (missing: {missing})"); sys.exit(1)
print("MORSE TEST PASSED  (SOS/HELLO encoded; word-gap slash between SOS SOS)")
PY
