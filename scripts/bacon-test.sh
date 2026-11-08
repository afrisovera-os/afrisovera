#!/usr/bin/env bash
# Verify `bacon <text>` encodes each ASCII letter as its 5-bit A/B Baconian code (26-letter, case-
# folded), non-letters passing through. "abc"->"AAAAAAAAABAAABA", "hi"->"AABBBABAAA", "OS"->
# "ABBBABAABA", "a!b"->"AAAAA!AAAAB". Self-checking: python 5-bit A/B code per letter.
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
OUT="${TMPDIR:-/tmp}/bacon.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/bacon.png" \
    "echo warmup" "bacon abc" "bacon hi" "bacon OS" "bacon a!b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bacon:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def bacon(t):                        # DERIVED reference: 5-bit A/B per letter, others passthrough
    out = []
    for ch in t:
        if ch.isascii() and ch.isalpha():
            idx = ord(ch.lower()) - ord("a")
            out.append("".join("B" if (idx >> b) & 1 else "A" for b in range(4, -1, -1)))
        else:
            out.append(ch)
    return "".join(out)

for t in ["abc", "hi", "OS", "a!b"]:
    want = "bacon: " + bacon(t)
    if want not in s:
        print(f"BACON TEST FAILED (missing '{want}')"); sys.exit(1)

assert bacon("abc") == "AAAAAAAAABAAABA"
assert bacon("hi") == "AABBBABAAA"
assert bacon("OS") == "ABBBABAABA"
assert bacon("a!b") == "AAAAA!AAAAB"
print("BACON TEST PASSED  (abc/hi/OS/a!b vs python 5-bit A/B codes)")
PY
