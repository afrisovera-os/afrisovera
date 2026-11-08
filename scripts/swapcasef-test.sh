#!/usr/bin/env bash
# Verify `swapcasef <file>` inverts the case of every letter in a VFS file: swapcasef motd -> the
# motd with each letter's case flipped. File analogue of swapcase (inline). Self-checking against the
# known seeded motd content via python str.swapcase(). Also checks the no-such-file path.
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
OUT="${TMPDIR:-/tmp}/swapcasef.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swapcasef.png" \
    "echo warmup" "swapcasef motd" "swapcasef nope.txt" "swapcasef" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapcasef:|usage: swapcasef" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# Known seeded content.
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
want = "swapcasef: " + motd.swapcase()   # DERIVED via python str.swapcase()

if want not in s:
    print(f"SWAPCASEF TEST FAILED (missing '{want}')"); sys.exit(1)
if "swapcasef: no such file: nope.txt" not in s:
    print("SWAPCASEF TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: swapcasef <file>" not in s:
    print("SWAPCASEF TEST FAILED (missing usage line)"); sys.exit(1)

print("SWAPCASEF TEST PASSED  (motd case-flipped vs python str.swapcase(), plus no-such-file + usage)")
PY
