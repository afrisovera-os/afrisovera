#!/usr/bin/env bash
# Verify the `anagram` command reports whether two words are anagrams (same letter
# multiset, case-insensitive, ignoring non-letters) -- distinct from hamming (per-
# position diff of equal-length strings). The harness compares sorted [a-z]-lowercased
# letters in python, so it is self-checking: listen/silent -> YES, hello/world -> NO,
# Dormitory/dirtyroom -> YES (case-insensitive), abc/abd -> NO; one word -> usage.
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
OUT="${TMPDIR:-/tmp}/anagram.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/anagram.png" \
    "anagram listen silent" "anagram hello world" "anagram Dormitory dirtyroom" \
    "anagram abc abd" "anagram foo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "anagram:|usage: anagram" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

def norm(w):
    return sorted(c for c in w.lower() if "a" <= c <= "z")

def expect(a, b):
    return "YES" if norm(a) == norm(b) else "NO"

# Verdicts appear in command order; only OUTPUT lines are "anagram: YES/NO"
# (echoes are "anagram <a> <b>"), so an ordered regex scan is unambiguous.
pairs = [("listen", "silent"), ("hello", "world"), ("Dormitory", "dirtyroom"), ("abc", "abd")]
want = [expect(a, b) for a, b in pairs]

got = re.findall(r"anagram: (YES|NO)", s)
if got != want:
    print(f"ANAGRAM TEST FAILED (verdict sequence {got} != expected {want})"); sys.exit(1)

if "usage: anagram <word1> <word2>" not in s:
    print("ANAGRAM TEST FAILED (missing usage line for single word)"); sys.exit(1)

print("ANAGRAM TEST PASSED  (listen/silent & Dormitory/dirtyroom->YES, hello/world & abc/abd->NO, 1 word->usage)")
PY
