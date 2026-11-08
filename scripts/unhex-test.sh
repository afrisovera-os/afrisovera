#!/usr/bin/env bash
# Verify the `unhex` command decodes a hex string to text (inverse of hexdump).
# Deterministic, so exact assertions prove it:
#   unhex 48656c6c6f  -> "unhex: Hello"
#   unhex 41 66 72 69 -> "unhex: Afri"   (optional spaces accepted)
#   unhex zz          -> "unhex: invalid hex"  (rejects malformed input)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4708,server,nowait \
    -qmp tcp:127.0.0.1:4709,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/unhex.out"
python3 scripts/shell-demo.py 4708 4709 "${TMPDIR:-/tmp}/unhex.png" "unhex 48656c6c6f" "unhex 41 66 72 69" "unhex zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unhex: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["unhex: Hello", "unhex: Afri", "unhex: invalid hex"]
missing = [w for w in want if w not in s]
if missing:
    print(f"UNHEX TEST FAILED (missing exact substring(s): {missing})"); sys.exit(1)
print("UNHEX TEST PASSED  (48656c6c6f->Hello; spaced 41 66 72 69->Afri; zz->invalid)")
PY
