#!/usr/bin/env bash
# Verify `lowersonly <text>` keeps ONLY the lowercase ASCII letters (a-z), dropping uppercase, digits,
# punctuation, and spaces -- the case mirror of uppersonly. Tests "AfriSovera OS project"->
# "frioveraproject", "Hello, World!"->"elloorld", all-uppercase -> (none). Self-checking: python [a-z].
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
OUT="${TMPDIR:-/tmp}/lowersonly.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lowersonly.png" \
    "echo warmup" "lowersonly AfriSovera OS project" "lowersonly Hello, World!" \
    "lowersonly ALL CAPS 123" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lowersonly:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lowers(t):                       # DERIVED reference: keep a-z only
    return "".join(c for c in t if "a" <= c <= "z")

cases = ["AfriSovera OS project", "Hello, World!"]
for t in cases:
    want = "lowersonly: " + lowers(t)
    if want not in s:
        print(f"LOWERSONLY TEST FAILED (missing '{want}')"); sys.exit(1)

assert lowers("AfriSovera OS project") == "frioveraproject"
assert lowers("Hello, World!") == "elloorld"
assert lowers("ALL CAPS 123") == ""
assert "lowersonly: (none)" in s
print("LOWERSONLY TEST PASSED  (frioveraproject / elloorld vs python [a-z]; all-uppercase -> (none))")
PY
