#!/usr/bin/env bash
# Verify the `charcount <ch> <file>` command counts how many times a single
# character (the FIRST char of <ch>) occurs across a VFS file's content.
# DISTINCT from grepc (counts matching LINES) and histogram (freq of ALL bytes).
# Self-checking: python loads the KNOWN about.txt seed constant and computes
# about.count(ch) for each tested char, asserting the exact output (case-sensitive).
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
OUT="${TMPDIR:-/tmp}/charcount.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/charcount.png" \
    "charcount o about.txt" "charcount e about.txt" "charcount a about.txt" \
    "charcount z about.txt" "charcount A about.txt" "charcount x nope.txt" \
    "charcount" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "charcount:|usage: charcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constant (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

for ch in ["o", "e", "a", "z", "A"]:
    want = f"charcount: {ch} occurs {about.count(ch)} times"
    if want not in s:
        print(f"CHARCOUNT TEST FAILED (char '{ch}': missing '{want}')"); sys.exit(1)

# sanity-pin (case-sensitive: 'a'=9 vs 'A'=3, 'z'=0)
assert about.count("o") == 9 and about.count("e") == 17
assert about.count("a") == 9 and about.count("A") == 3 and about.count("z") == 0

if "charcount: no such file: nope.txt" not in s:
    print("CHARCOUNT TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: charcount <ch> <file>" not in s:
    print("CHARCOUNT TEST FAILED (missing usage line)"); sys.exit(1)

print("CHARCOUNT TEST PASSED  (counts exact vs seed text, case-sensitive, no-file/usage)")
PY
