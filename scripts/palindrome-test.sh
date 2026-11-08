#!/usr/bin/env bash
# Verify the `palindrome` command reports whether text reads the same forwards and
# backwards, ignoring case and non-alphanumeric characters (distinct from revnum,
# which is numeric-only). The harness normalizes each phrase the same way in
# python (keep [a-z0-9] after lowercasing) so it is self-checking: racecar/12321/
# abccba/"Able was I ere I saw Elba" -> YES; hello/ab -> NO; empty -> usage.
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
OUT="${TMPDIR:-/tmp}/palindrome.out"
# Commands are sent in order; each echo delimits its output region so YES/NO
# verdicts can be attributed to the right phrase (the verdicts alone are ambiguous).
CMDS=("palindrome racecar" "palindrome hello" "palindrome 12321" "palindrome abccba" \
      "palindrome ab" "palindrome Able was I ere I saw Elba" "palindrome")
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/palindrome.png" "${CMDS[@]}" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "palindrome:|usage: palindrome" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

def expect(text):
    n = [c for c in text.lower() if c.isalnum()]
    return "YES" if n == n[::-1] else "NO"

# The verdicts appear in command order. Only the OUTPUT lines contain
# "palindrome: YES/NO" (echoes are "palindrome <phrase>", no colon), so an ordered
# regex scan yields exactly the verdict sequence -- robust without echo-segmenting.
phrases = ["racecar", "hello", "12321", "abccba", "ab", "Able was I ere I saw Elba"]
want = [expect(p) for p in phrases]

got = re.findall(r"palindrome: (YES|NO)", s)
if got != want:
    print(f"PALINDROME TEST FAILED (verdict sequence {got} != expected {want})"); sys.exit(1)

if "usage: palindrome <text>" not in s:
    print("PALINDROME TEST FAILED (missing usage line)"); sys.exit(1)

print("PALINDROME TEST PASSED  (racecar/12321/abccba/Elba->YES, hello/ab->NO, empty->usage)")
PY
