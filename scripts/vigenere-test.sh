#!/usr/bin/env bash
# Verify the `vigenere` command applies the repeating-key Vigenere cipher: the
# canonical KEY/HELLO -> RIJVS, lowercase key/hello -> rijvs, and the textbook
# LEMON/ATTACKATDAWN -> LXFOPVEFRNHR. The test recomputes each expected value in
# python for a self-checking assert.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4882,server,nowait \
    -qmp tcp:127.0.0.1:4883,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/vigenere.out"
python3 scripts/shell-demo.py 4882 4883 "${TMPDIR:-/tmp}/vigenere.png" \
    "vigenere KEY HELLO" "vigenere key hello" "vigenere LEMON ATTACKATDAWN" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vigenere:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def vig(key, text):
    out = ""; ki = 0; k = [ord(c.lower()) - 97 for c in key]
    for c in text:
        if 'A' <= c <= 'Z':
            out += chr((ord(c) - 65 + k[ki % len(k)]) % 26 + 65); ki += 1
        elif 'a' <= c <= 'z':
            out += chr((ord(c) - 97 + k[ki % len(k)]) % 26 + 97); ki += 1
        else:
            out += c
    return out

cases = [("KEY", "HELLO"), ("key", "hello"), ("LEMON", "ATTACKATDAWN")]
for key, text in cases:
    want = f"vigenere: {vig(key, text)}"
    if want not in s:
        print(f"VIGENERE TEST FAILED (expected '{want}' for {key}/{text})"); sys.exit(1)

print("VIGENERE TEST PASSED  (KEY/HELLO->RIJVS, key/hello->rijvs, LEMON/ATTACKATDAWN->LXFOPVEFRNHR)")
PY
