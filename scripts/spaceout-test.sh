#!/usr/bin/env bash
# Verify the `spaceout <text>` command inserts a single space between each
# character of the inline text (letter-spacing). e.g. "abc" -> "a b c",
# "Hi!" -> "H i !". The conceptual inverse of despace (which removes whitespace);
# distinct from squeeze and the case/cipher transforms. Self-checking: python's
# ' '.join(list(t)) is the reference.
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
OUT="${TMPDIR:-/tmp}/spaceout.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/spaceout.png" \
    "echo warmup" "spaceout abc" "spaceout Hi!" "spaceout x" \
    "spaceout" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "spaceout:|usage: spaceout" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def so(t):
    return ' '.join(list(t))

cases = ["abc", "Hi!", "x"]
for text in cases:
    want = f"spaceout: {so(text)}"
    if want not in s:
        print(f"SPACEOUT TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: a space between every character; a single char is unchanged
assert so("abc") == "a b c"
assert so("Hi!") == "H i !"
assert so("x") == "x"

if "usage: spaceout <text>" not in s:
    print("SPACEOUT TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("SPACEOUT TEST PASSED  (abc->'a b c', Hi!->'H i !', x->'x' exact vs python; "
      "empty->usage)")
PY
