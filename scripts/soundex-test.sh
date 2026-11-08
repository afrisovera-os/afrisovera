#!/usr/bin/env bash
# Verify the `soundex` command computes the Soundex phonetic code (first letter +
# 3 digits) using the standard h/w-aware algorithm. Cross-checked two ways: (a) a
# python re-implementation of the SAME algorithm, and (b) hard-pinned CANONICAL
# published values (Robert/Rupert=R163, Ashcraft=A261 (h/w rule), Tymczak=T522,
# Honeyman=H555) so a matching bug in both impls cannot hide.
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
OUT="${TMPDIR:-/tmp}/soundex.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/soundex.png" \
    "soundex Robert" "soundex Rupert" "soundex Ashcraft" "soundex Tymczak" \
    "soundex Honeyman" "soundex" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "soundex:|usage: soundex" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def digit(c):
    return {**dict.fromkeys("bfpv", 1), **dict.fromkeys("cgjkqsxz", 2),
            **dict.fromkeys("dt", 3), "l": 4, **dict.fromkeys("mn", 5),
            "r": 6}.get(c, 0)

def soundex(word):
    letters = [ch.lower() for ch in word if ch.isalpha()]
    out = [letters[0].upper()]
    prev = digit(letters[0])
    for ch in letters[1:]:
        code = digit(ch)
        if code != 0 and code != prev and len(out) < 4:
            out.append(str(code))
        if ch not in ("h", "w"):
            prev = code
    return "".join(out).ljust(4, "0")[:4]

# (a) algorithm cross-check; (b) hard-pinned canonical constants.
canonical = {"Robert": "R163", "Rupert": "R163", "Ashcraft": "A261",
             "Tymczak": "T522", "Honeyman": "H555"}
for word, canon in canonical.items():
    assert soundex(word) == canon, f"python impl {soundex(word)} != canonical {canon} for {word}"
    want = f"soundex: {canon}"
    if want not in s:
        print(f"SOUNDEX TEST FAILED (missing '{want}' for {word})"); sys.exit(1)

if "usage: soundex <word>" not in s:
    print("SOUNDEX TEST FAILED (missing usage line)"); sys.exit(1)

print("SOUNDEX TEST PASSED  (Robert/Rupert=R163, Ashcraft=A261, Tymczak=T522, Honeyman=H555; empty->usage)")
PY
