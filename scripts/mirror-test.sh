#!/usr/bin/env bash
# Verify the `mirror <text>` command appends the reverse of the text to itself, building
# an (even-length) palindrome: "abc"->"abccba", "ab"->"abba", "a"->"aa". Distinct from
# rev (reverses the whole string), double (repeats each char twice -> "aabbcc"),
# palindrome (classifies), and repeat (repeats the whole string n times). Self-checking:
# python replicates t + t[::-1] and asserts the result is a palindrome.
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
OUT="${TMPDIR:-/tmp}/mirror.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "a" pins the single-char case ("aa"); "hello" a longer word.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mirror.png" \
    "echo warmup" "mirror abc" "mirror ab" "mirror a" "mirror hello" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mirror:|usage: mirror" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def mirror(t):
    return t + t[::-1]

for t in ["abc", "ab", "a", "hello"]:
    m = mirror(t)
    assert m == m[::-1], f"internal: {m} not a palindrome"  # mirror always yields a palindrome
    want = f"mirror: {m}"
    if want not in s:
        print(f"MIRROR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "mirror: abccba" in s
assert "mirror: abba" in s
assert "mirror: aa" in s               # single char -> doubled
assert "mirror: helloolleh" in s

print("MIRROR TEST PASSED  (abc->abccba, ab->abba, a->aa, hello->helloolleh vs python; "
      "all palindromes; text+reverse, distinct from rev/double/palindrome)")
PY
