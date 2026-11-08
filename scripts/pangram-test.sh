#!/usr/bin/env bash
# Verify the `pangram` command reports whether inline text contains every letter
# A-Z (case-insensitive), listing missing letters otherwise. The harness computes
# the expected result for each phrase with the same lowercase-mask rule in python,
# so it is self-checking: the classic fox pangram -> YES; "hello world" -> NO with
# the exact missing set; a-z -> YES; empty -> usage.
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
OUT="${TMPDIR:-/tmp}/pangram.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pangram.png" \
    "pangram The quick brown fox jumps over the lazy dog" "pangram hello world" \
    "pangram abcdefghijklmnopqrstuvwxyz" "pangram" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pangram:|usage: pangram" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, string
s = open(sys.argv[1], "rb").read().decode("latin1")

def expect(text):
    present = set(c for c in text.lower() if "a" <= c <= "z")
    missing = "".join(ch for ch in string.ascii_lowercase if ch not in present)
    if not missing:
        return "pangram: YES (all 26 letters present)"
    return f"pangram: NO (missing: {missing})"

phrases = [
    "The quick brown fox jumps over the lazy dog",
    "hello world",
    "abcdefghijklmnopqrstuvwxyz",
]
for p in phrases:
    want = expect(p)
    if want not in s:
        print(f"PANGRAM TEST FAILED (missing '{want}' for {p!r})"); sys.exit(1)

if "usage: pangram <text>" not in s:
    print("PANGRAM TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("PANGRAM TEST PASSED  (fox->YES, 'hello world'->NO missing abcfgijkmnpqstuvxyz, a-z->YES, empty->usage)")
PY
