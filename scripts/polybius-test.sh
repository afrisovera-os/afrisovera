#!/usr/bin/env bash
# Verify `polybius <text>` encodes each letter as its 5x5 Polybius-square row-col coordinates (I/J
# share 24), space-separated, non-letters skipped, case-folded. "abc"->"11 12 13", "hello"->"23 15 31
# 31 34", "jazz"->"24 11 55 55", "OS"->"34 43". Self-checking: python Polybius mapping.
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
OUT="${TMPDIR:-/tmp}/polybius.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/polybius.png" \
    "echo warmup" "polybius abc" "polybius hello" "polybius jazz" "polybius OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "polybius:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def polybius(t):                     # DERIVED reference
    codes = []
    for ch in t:
        if ch.isascii() and ch.isalpha():
            c = ch.lower()
            if c == "j": c = "i"
            idx = ord(c) - ord("a")
            if c > "i": idx -= 1        # j removed from the 25-letter alphabet
            codes.append(f"{idx // 5 + 1}{idx % 5 + 1}")
    return " ".join(codes)

for t in ["abc", "hello", "jazz", "OS"]:
    want = "polybius: " + polybius(t)
    if want not in s:
        print(f"POLYBIUS TEST FAILED (missing '{want}')"); sys.exit(1)

assert polybius("abc") == "11 12 13"
assert polybius("hello") == "23 15 31 31 34"
assert polybius("jazz") == "24 11 55 55"
assert polybius("OS") == "34 43"
print("POLYBIUS TEST PASSED  (11 12 13 / 23 15 31 31 34 / 24 11 55 55 / 34 43 vs python)")
PY
