#!/usr/bin/env bash
# Verify `uppersonly <text>` keeps ONLY the uppercase ASCII letters (A-Z), dropping lowercase,
# digits, punctuation, and spaces -- an "extract the capitals" filter. Tests "AfriSovera OS project"
# ->"ASOS", "Hello, World!"->"HW", all-lowercase -> (none). Self-checking: python keeps [A-Z].
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
OUT="${TMPDIR:-/tmp}/uppersonly.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/uppersonly.png" \
    "echo warmup" "uppersonly AfriSovera OS project" "uppersonly Hello, World!" \
    "uppersonly all lower 123" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uppersonly:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def uppers(t):                       # DERIVED reference: keep A-Z only
    return "".join(c for c in t if "A" <= c <= "Z")

cases = ["AfriSovera OS project", "Hello, World!"]
for t in cases:
    want = "uppersonly: " + uppers(t)
    if want not in s:
        print(f"UPPERSONLY TEST FAILED (missing '{want}')"); sys.exit(1)

assert uppers("AfriSovera OS project") == "ASOS"
assert uppers("Hello, World!") == "HW"
assert uppers("all lower 123") == ""
assert "uppersonly: (none)" in s
print("UPPERSONLY TEST PASSED  (ASOS / HW vs python [A-Z]; all-lowercase -> (none))")
PY
