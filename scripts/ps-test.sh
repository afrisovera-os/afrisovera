#!/usr/bin/env bash
# Verify the `ps` command introspects the kernel process table. Asserts the
# single-line summary (robust to capture truncation): N_PROC == 6 slots, the
# state counts sum to the slot count, and the shell stays alive afterwards
# (uptime prints). Read-only -- must not disturb the process subsystem.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4652,server,nowait \
    -qmp tcp:127.0.0.1:4653,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/ps.out"
python3 scripts/shell-demo.py 4652 4653 "${TMPDIR:-/tmp}/ps.png" "ps" "uptime" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "SLOT|ps:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"ps: (\d+) slots, (\d+) runnable, (\d+) blocked, (\d+) dead", s)
if not m:
    print("PS TEST FAILED (no ps summary line)"); sys.exit(1)
slots, r, b, d = (int(x) for x in m.groups())
if slots != 6:
    print(f"PS TEST FAILED (expected 6 slots, got {slots})"); sys.exit(1)
if r + b + d != slots:
    print(f"PS TEST FAILED (state counts {r}+{b}+{d} != {slots} slots)"); sys.exit(1)
if r < 1:
    print("PS TEST FAILED (no runnable slot -- the idle/shell context should be runnable)"); sys.exit(1)
# shell must still respond after the ps block
if "uptime" not in s.split("ps:", 1)[-1]:
    print("PS TEST FAILED (shell did not respond after ps)"); sys.exit(1)
print(f"PS TEST PASSED  (process table: {slots} slots, {r} runnable, {b} blocked, {d} dead; shell alive)")
PY
