#!/usr/bin/env bash
# Verify the `grepc <pattern> <file>` command counts how many lines of a VFS file
# contain the substring pattern (like grep -c), completing the grep family (grep
# prints, grepn numbers, grepv inverts, grepc counts). Self-checking: python
# loads the KNOWN seed constant for about.txt and computes the expected count for
# each pattern, then asserts the exact `grepc: <n> matching lines` output.
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
OUT="${TMPDIR:-/tmp}/grepc.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/grepc.png" \
    "grepc the about.txt" "grepc o about.txt" "grepc AfriSovera about.txt" \
    "grepc zzz about.txt" "grepc x nope.txt" "grepc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "grepc:|usage: grepc" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constant (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def grepc(text, pat):
    return sum(1 for line in text.split("\n") if pat in line)

for pat in ["the", "o", "AfriSovera", "zzz"]:
    want = f"grepc: {grepc(about, pat)} matching lines"
    if want not in s:
        print(f"GREPC TEST FAILED (pattern '{pat}': missing '{want}')"); sys.exit(1)

# sanity-pin the expected counts against the seed text
assert grepc(about, "the") == 1     # only line 2 ("the device", "the software")
assert grepc(about, "o") == 2       # both lines contain 'o'
assert grepc(about, "AfriSovera") == 1
assert grepc(about, "zzz") == 0

if "grepc: no such file: nope.txt" not in s:
    print("GREPC TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: grepc <pattern> <file>" not in s:
    print("GREPC TEST FAILED (missing usage line)"); sys.exit(1)

print("GREPC TEST PASSED  (counts exact vs seed text, no-such-file/usage)")
PY
