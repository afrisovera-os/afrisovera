#!/usr/bin/env bash
# Verify the `unrle <rle>` command DECODES run-length encoding back to text -- the
# inverse of `rle`, which emits <char><decimal-count> pairs (every run counted).
# unrle reads a literal char then its following decimal digits as the count and
# repeats the char that many times. NOTE: char+count RLE is only unambiguously
# decodable for DIGIT-FREE source text (so the round-trip cases below use
# digit-free strings). Self-checking: python builds the SAME rle format, ENCODES
# known strings, feeds them to unrle, and asserts unrle(rle(x)) == x (round-trip),
# including a >=10 (multi-digit) run.
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
OUT="${TMPDIR:-/tmp}/unrle.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/unrle.png" \
    "unrle a3b2c1" "unrle h1e1l2o1" "unrle x12" "unrle" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unrle:|usage: unrle" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rle(text):  # mirror the kernel's rle encoder (<char><count>, every run)
    out = []
    i = 0
    while i < len(text):
        j = i
        while j < len(text) and text[j] == text[i]:
            j += 1
        out.append(text[i] + str(j - i))
        i = j
    return "".join(out)

# Round-trip: unrle(rle(x)) == x for DIGIT-FREE inputs (incl. a >=10 run).
for original in ["aaabbc", "hello", "x" * 12]:
    code = rle(original)
    want = f"unrle: {original}"
    if want not in s:
        print(f"UNRLE TEST FAILED (round-trip for {original!r} via '{code}': missing '{want}')")
        sys.exit(1)

if "usage: unrle <rle>" not in s:
    print("UNRLE TEST FAILED (missing usage line)"); sys.exit(1)

print("UNRLE TEST PASSED  (decode inverts rle: unrle(rle(x))==x incl. multi-digit run, empty->usage)")
PY
