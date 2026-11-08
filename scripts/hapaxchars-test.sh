#!/usr/bin/env bash
# Verify `hapaxchars <text>` lists ALL characters occurring EXACTLY ONCE (case-sensitive) in first-
# occurrence order -- the char-level analogue of hapax. "mississippi"->"m", "hello"->"heo",
# "swiss"->"wi", all-repeated -> (none). Self-checking: python Counter over characters.
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
OUT="${TMPDIR:-/tmp}/hapaxchars.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hapaxchars.png" \
    "echo warmup" "hapaxchars mississippi" "hapaxchars hello" "hapaxchars swiss" \
    "hapaxchars aabbcc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hapaxchars:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from collections import Counter
s = open(sys.argv[1], "rb").read().decode("latin1")

def hapaxchars(t):                   # DERIVED reference: chars with count==1, first-occurrence order
    c = Counter(t)
    seen, out = set(), []
    for ch in t:
        if c[ch] == 1 and ch not in seen:
            seen.add(ch); out.append(ch)
    return "".join(out)

cases = {"mississippi": "m", "hello": "heo", "swiss": "wi"}
for t, exp in cases.items():
    assert hapaxchars(t) == exp, (t, hapaxchars(t))
    want = "hapaxchars: " + exp
    if want not in s:
        print(f"HAPAXCHARS TEST FAILED (missing '{want}')"); sys.exit(1)

assert hapaxchars("aabbcc") == ""
if "hapaxchars: (none)" not in s:
    print("HAPAXCHARS TEST FAILED (missing '(none)' for all-repeated)"); sys.exit(1)

print("HAPAXCHARS TEST PASSED  (m / heo / wi / (none) vs python Counter count==1)")
PY
