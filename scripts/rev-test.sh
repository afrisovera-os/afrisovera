#!/usr/bin/env bash
# Verify the `rev` command reverses text in byte order (correct for ASCII).
# Deterministic, so exact-substring assertions prove it (non-palindrome inputs):
#   rev AfriSovera -> "rev: arevoSirfA"
#   rev abcdef     -> "rev: fedcba"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4706,server,nowait \
    -qmp tcp:127.0.0.1:4707,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/rev.out"
python3 scripts/shell-demo.py 4706 4707 "${TMPDIR:-/tmp}/rev.png" "rev AfriSovera" "rev abcdef" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rev: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["rev: arevoSirfA", "rev: fedcba"]
missing = [w for w in want if w not in s]
if missing:
    print(f"REV TEST FAILED (missing exact substring(s): {missing})"); sys.exit(1)
print("REV TEST PASSED  (AfriSovera -> arevoSirfA; abcdef -> fedcba)")
PY
