#!/usr/bin/env bash
# Verify `debacon <text>` decodes a Baconian cipher back to (lowercase) letters -- the inverse of
# bacon. Groups of 5 A/B -> a letter; other bytes flush partial runs and pass through.
# "AAAAAAAAABAAABA"->"abc", "AABBBABAAA"->"hi", "AAAAA!AAAAB"->"a!b". Self-checking: python decode.
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
OUT="${TMPDIR:-/tmp}/debacon.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/debacon.png" \
    "echo warmup" "debacon AAAAAAAAABAAABA" "debacon AABBBABAAA" \
    "debacon AAAAA!AAAAB" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "debacon:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def debacon(t):                      # DERIVED reference: groups of 5 A/B -> lowercase letter
    out, grp = [], []
    for ch in t:
        if ch in "AB":
            grp.append(ch)
            if len(grp) == 5:
                val = int("".join("1" if c == "B" else "0" for c in grp), 2)
                out.append(chr(ord("a") + val) if val <= 25 else "?")
                grp = []
        else:
            out.extend(grp); grp = []
            out.append(ch)
    out.extend(grp)
    return "".join(out)

def bacon(t):
    o = []
    for ch in t:
        if ch.isascii() and ch.isalpha():
            idx = ord(ch.lower()) - ord("a")
            o.append("".join("B" if (idx >> b) & 1 else "A" for b in range(4, -1, -1)))
        else:
            o.append(ch)
    return "".join(o)

cases = ["AAAAAAAAABAAABA", "AABBBABAAA", "AAAAA!AAAAB"]
for t in cases:
    want = "debacon: " + debacon(t)
    if want not in s:
        print(f"DEBACON TEST FAILED (missing '{want}')"); sys.exit(1)

assert debacon("AAAAAAAAABAAABA") == "abc"
assert debacon("AABBBABAAA") == "hi"
assert debacon("AAAAA!AAAAB") == "a!b"
# round-trip: debacon(bacon(x)) restores x lowercased (letters) / verbatim (non-letters)
assert debacon(bacon("a!b")) == "a!b"
print("DEBACON TEST PASSED  (abc/hi/a!b vs python; debacon(bacon(x)) round-trips)")
PY
