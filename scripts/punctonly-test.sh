#!/usr/bin/env bash
# Verify `punctonly <text>` keeps ONLY ASCII punctuation/symbol bytes, dropping letters, digits, and
# spaces -- the exact complement of strippunct (which drops punctuation, keeps alphanumerics+spaces).
# Tests "phone: 555-0199"->":-", "a-b_c 1!2"->"-_!", and an all-alnum input -> (none). Self-checking:
# python keeps string.punctuation.
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
OUT="${TMPDIR:-/tmp}/punctonly.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/punctonly.png" \
    "echo warmup" 'punctonly phone: 555-0199' 'punctonly a-b_c 1!2' \
    'punctonly hello world 123' > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "punctonly:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, string
s = open(sys.argv[1], "rb").read().decode("latin1")

def punctonly(t):                    # DERIVED reference: keep ASCII punctuation only
    return "".join(c for c in t if c in string.punctuation)

cases = ["phone: 555-0199", "a-b_c 1!2"]
for t in cases:
    want = "punctonly: " + punctonly(t)
    if want not in s:
        print(f"PUNCTONLY TEST FAILED (missing '{want}')"); sys.exit(1)

assert punctonly("phone: 555-0199") == ":-"
assert punctonly("a-b_c 1!2") == "-_!"
assert punctonly("hello world 123") == ""     # all alnum/space -> (none)
assert "punctonly: (none)" in s
print("PUNCTONLY TEST PASSED  (:- , -_! vs python string.punctuation; all-alnum -> (none))")
PY
