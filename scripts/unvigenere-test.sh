#!/usr/bin/env bash
# Verify `unvigenere <key> <text>` decrypts the Vigenere cipher (subtract key shift, key advances only
# on letters, non-letters passthrough) -- the inverse of vigenere. Tests the classic
# "unvigenere lemon LXFOPVEFRNHR" -> "ATTACKATDAWN", "unvigenere b bcd" -> "abc", and a mixed-case
# round-trip. Self-checking: python Vigenere decrypt.
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
OUT="${TMPDIR:-/tmp}/unvigenere.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/unvigenere.png" \
    "echo warmup" "unvigenere lemon LXFOPVEFRNHR" "unvigenere b bcd" \
    "unvigenere key Rijvs" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unvigenere:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dec(key, text):                  # DERIVED reference: Vigenere decrypt
    out, ki = [], 0
    for c in text:
        if c.isalpha() and c.isascii():
            sh = ord(key[ki % len(key)].lower()) - ord("a"); ki += 1
            base = ord("A") if c.isupper() else ord("a")
            out.append(chr(base + (ord(c) - base - sh) % 26))
        else:
            out.append(c)
    return "".join(out)

cases = [("lemon", "LXFOPVEFRNHR"), ("b", "bcd"), ("key", "Rijvs")]
for key, ct in cases:
    want = "unvigenere: " + dec(key, ct)
    if want not in s:
        print(f"UNVIGENERE TEST FAILED (missing '{want}')"); sys.exit(1)

assert dec("lemon", "LXFOPVEFRNHR") == "ATTACKATDAWN"
assert dec("b", "bcd") == "abc"
print("UNVIGENERE TEST PASSED  (lemon->ATTACKATDAWN, b->abc, key->Hello vs python decrypt)")
PY
