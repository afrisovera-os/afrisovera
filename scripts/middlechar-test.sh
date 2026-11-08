#!/usr/bin/env bash
# Verify the `middlechar <text>` command emits the middle character(s) of the
# inline text, delimited by [ ]: an odd length gives the single middle char
# ("hello" -> "l"), an even length gives the two middle chars ("test" -> "es",
# "AfriSovera" -> "So"). Distinct from charat (a char at an explicit index of a
# VFS file), firstn/lastn (prefix/suffix), and chars (class breakdown). Self-
# checking: python computes the center indices.
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
OUT="${TMPDIR:-/tmp}/middlechar.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "a" pins the single-char case; the odd/even split is the key behaviour.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/middlechar.png" \
    "echo warmup" "middlechar hello" "middlechar test" "middlechar a" "middlechar" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "middlechar:|usage: middlechar" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def mid(t):
    n = len(t)
    return t[(n - 1) // 2:(n - 1) // 2 + 1] if n % 2 else t[n // 2 - 1:n // 2 + 1]

for t in ["hello", "test", "a"]:
    want = f"middlechar: [{mid(t)}]"
    if want not in s:
        print(f"MIDDLECHAR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "middlechar: [l]" in s    # odd length 5 -> single middle char
assert "middlechar: [es]" in s   # even length 4 -> two middle chars
assert "middlechar: [a]" in s    # single char -> itself

# empty input -> usage
if "usage: middlechar <text>" not in s:
    print("MIDDLECHAR TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("MIDDLECHAR TEST PASSED  (odd hello->l (1 char), even test->es (2 chars), a->a "
      "vs python; empty -> usage; distinct from charat/firstn/lastn)")
PY
