#!/usr/bin/env bash
# Verify the `railfence <text>` command applies the RAIL FENCE (zigzag)
# TRANSPOSITION cipher with 3 rails: write bytes in a zigzag across 3 rails
# (rail of position i follows the period-4 pattern 0,1,2,1,...) then read off
# rail-by-rail. Unlike caesar/atbash/rot47/vigenere/leet (substitution -- MAP
# chars), rail fence REORDERS them -- the output is a permutation of the input.
# Self-checking: python replicates the 3-rail encode and matches each output.
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
OUT="${TMPDIR:-/tmp}/railfence.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/railfence.png" \
    "echo warmup" "railfence WEAREDISCOVEREDFLEEATONCE" "railfence HELLO" \
    "railfence ABC" "railfence" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "railfence:|usage: railfence" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rf(t):
    rails = ['', '', '']
    for i, ch in enumerate(t):
        c = i % 4
        r = c if c <= 2 else 4 - c
        rails[r] += ch
    return ''.join(rails)

cases = ["WEAREDISCOVEREDFLEEATONCE", "HELLO", "ABC"]
for text in cases:
    want = f"railfence: {rf(text)}"
    if want not in s:
        print(f"RAILFENCE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the classic 3-rail result + length-preserving permutation property
assert rf("WEAREDISCOVEREDFLEEATONCE") == "WECRLTEERDSOEEFEAOCAIVDEN"
assert rf("HELLO") == "HOELL"
assert rf("ABC") == "ABC"
for text in cases:
    assert sorted(rf(text)) == sorted(text)   # a permutation of the input

if "usage: railfence <text>" not in s:
    print("RAILFENCE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("RAILFENCE TEST PASSED  (3-rail zigzag exact vs python incl the classic "
      "WEARE... case; permutation of input; empty->usage)")
PY
