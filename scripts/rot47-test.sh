#!/usr/bin/env bash
# Verify the `rot47` command applies the ROT47 cipher (rotate printable ASCII
# 33..126 by 47; self-inverse): hello -> 96==@, AfriSovera -> p7C:$@G6C2, and the
# round-trip rot47(96==@) -> hello. Output has $/% chars, so use fixed-string checks.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4938,server,nowait \
    -qmp tcp:127.0.0.1:4939,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/rot47.out"
python3 scripts/shell-demo.py 4938 4939 "${TMPDIR:-/tmp}/rot47.png" \
    "rot47 hello" "rot47 AfriSovera" "rot47 96==@" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aF "rot47:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def r47(t):
    return "".join(chr(33 + (ord(c) - 33 + 47) % 94) if 33 <= ord(c) <= 126 else c for c in t)

# Fixed-string substring checks (output contains $ / % / regex metachars).
cases = ["hello", "AfriSovera", "96==@"]  # 3rd is the round-trip of the 1st
for text in cases:
    want = f"rot47: {r47(text)}"
    if want not in s:
        print(f"ROT47 TEST FAILED (expected {want!r} for input {text!r})"); sys.exit(1)

# Explicit self-inverse: rot47(96==@) must be hello.
if r47("96==@") != "hello":
    print("ROT47 TEST FAILED (self-inverse property broken)"); sys.exit(1)

print("ROT47 TEST PASSED  (hello<->96==@ round-trip, AfriSovera->p7C:$@G6C2; self-inverse)")
PY
