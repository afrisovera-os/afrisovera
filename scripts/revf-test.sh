#!/usr/bin/env bash
# Verify the `revf` command prints each line of a VFS file with its characters
# reversed (the Unix `rev` command, on a file) -- distinct from `rev` (inline args)
# and `tac` (reverses line ORDER). Uses the seeded single-line motd. The harness
# computes the expected reversal in python (motd[::-1]), so it is self-checking.
# Also checks the no-such-file and usage paths.
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
OUT="${TMPDIR:-/tmp}/revf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/revf.png" \
    "revf motd" "revf nope.txt" "revf" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "arevoSirfA|no such file|usage: revf" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded motd is a known constant; the expected output is its reversal.
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
want = motd[::-1]
if want not in s:
    print(f"REVF TEST FAILED (missing reversed motd {want!r})"); sys.exit(1)

if "revf: no such file: nope.txt" not in s:
    print("REVF TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: revf <file>" not in s:
    print("REVF TEST FAILED (missing usage line)"); sys.exit(1)

print("REVF TEST PASSED  (motd reversed char-by-char vs python motd[::-1]; no-such-file + usage handled)")
PY
