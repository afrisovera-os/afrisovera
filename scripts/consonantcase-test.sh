#!/usr/bin/env bash
# Verify `consonantcase <text>` uppercases every consonant and lowercases every vowel (non-letters
# unchanged) -- the exact complement of vowelcase: "hello world"->"HeLLo WoRLD", "AEIOU"->"aeiou",
# "BCDFG"->"BCDFG", "Sankofa UI"->"SaNKoFa ui". Self-checking: python applies the same rule.
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
OUT="${TMPDIR:-/tmp}/consonantcase.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/consonantcase.png" \
    "echo warmup" "consonantcase hello world" "consonantcase AEIOU" "consonantcase BCDFG" \
    "consonantcase Sankofa UI" "consonantcase Own your device" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "consonantcase:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def cc(t):                            # DERIVED reference: consonants->upper, vowels->lower
    out = []
    for ch in t:
        if ch.isascii() and ch.isalpha():
            out.append(ch.lower() if ch.lower() in "aeiou" else ch.upper())
        else:
            out.append(ch)
    return "consonantcase: " + "".join(out)

for t in ("hello world", "AEIOU", "BCDFG", "Sankofa UI", "Own your device"):
    w = cc(t)
    if w not in s:
        print(f"CONSONANTCASE TEST FAILED (missing '{w}')"); sys.exit(1)

assert "consonantcase: HeLLo WoRLD" in s
assert "consonantcase: aeiou" in s
assert "consonantcase: BCDFG" in s
assert "consonantcase: SaNKoFa ui" in s
assert "consonantcase: oWN YouR DeViCe" in s

print("CONSONANTCASE TEST PASSED  (hello world->HeLLo WoRLD, AEIOU->aeiou, BCDFG->BCDFG, "
      "Sankofa UI->SaNKoFa ui, Own your device->oWN YouR DeViCe vs python complement-of-vowelcase)")
PY
