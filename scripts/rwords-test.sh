#!/usr/bin/env bash
# Verify the `rwords` command reverses the ORDER of words (not the characters
# within each word -- that's `rev`). Deterministic:
#   rwords the quick brown fox -> "fox brown quick the"
#   rwords AfriSovera OS       -> "OS AfriSovera"
#   rwords one                 -> "one"  (single word unchanged)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4792,server,nowait \
    -qmp tcp:127.0.0.1:4793,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/rwords.out"
python3 scripts/shell-demo.py 4792 4793 "${TMPDIR:-/tmp}/rwords.png" "rwords the quick brown fox" "rwords AfriSovera OS" "rwords one" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rwords: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["rwords: fox brown quick the", "rwords: OS AfriSovera", "rwords: one"]
missing = [w for w in want if w not in s]
if missing:
    print(f"RWORDS TEST FAILED (missing: {missing})"); sys.exit(1)
print("RWORDS TEST PASSED  (word order reversed; single word unchanged)")
PY
