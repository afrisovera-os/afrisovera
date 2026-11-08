#!/usr/bin/env bash
# Verify the `isogram <text>` command reports whether every alphabetic character is
# DISTINCT (no letter repeats), case-insensitive, ignoring non-letters. "black" and
# "subdermatoglyphic" ARE; "hello" (l twice) and "AfriSovera" (r repeats) are NOT.
# Distinct from pangram (all 26 letters present), uniqchars (removes dup chars), anagram.
# Self-checking: python replicates the first-repeat detection.
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
OUT="${TMPDIR:-/tmp}/isogram.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "hello" pins the repeated-letter reason ('l'); "subdermatoglyphic" a long true isogram.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isogram.png" \
    "echo warmup" "isogram black" "isogram hello" "isogram AfriSovera" "isogram subdermatoglyphic" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isogram:|usage: isogram" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(t):
    seen = set()
    repeated = None
    for ch in t.lower():
        if "a" <= ch <= "z":
            if ch in seen:
                if repeated is None:
                    repeated = ch
            else:
                seen.add(ch)
    if repeated is None:
        return "isogram: YES (all letters distinct)"
    return f"isogram: NO (letter '{repeated}' repeats)"

for t in ["black", "hello", "AfriSovera", "subdermatoglyphic"]:
    want = line(t)
    if want not in s:
        print(f"ISOGRAM TEST FAILED (missing '{want}')"); sys.exit(1)

# pin both output shapes + the named repeated letter
assert "isogram: YES (all letters distinct)" in s
assert "isogram: NO (letter 'l' repeats)" in s        # hello
assert "isogram: NO (letter 'r' repeats)" in s        # AfriSovera (case-insensitive)

print("ISOGRAM TEST PASSED  (black/subdermatoglyphic YES, hello NO ('l'), AfriSovera NO ('r') "
      "vs python; 26-bit seen-mask, distinct from pangram/uniqchars)")
PY
