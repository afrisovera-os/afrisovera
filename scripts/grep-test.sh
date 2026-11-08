#!/usr/bin/env bash
# Verify the `grep` command: a matching pattern prints the line, a non-matching
# one prints "(no matches)". RAM-only.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4660,server,nowait \
    -qmp tcp:127.0.0.1:4661,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/grep.out"
python3 scripts/shell-demo.py 4660 4661 "${TMPDIR:-/tmp}/grep.png" \
    "write log ERROR-42-CRASH" "grep ERROR log" "grep XYZ log" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "ERROR-42-CRASH|no matches|grep" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
after_match = s.split("grep ERROR log", 1)[-1]
ok_match = "ERROR-42-CRASH" in after_match
ok_none = "(no matches)" in s
if ok_match and ok_none:
    print("GREP TEST PASSED  (matching line printed; non-match reported as no matches)")
    sys.exit(0)
print(f"GREP TEST FAILED (match={ok_match} no_match={ok_none})")
sys.exit(1)
PY
