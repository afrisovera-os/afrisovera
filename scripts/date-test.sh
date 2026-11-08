#!/usr/bin/env bash
# Verify the `date` command reads the PL031 RTC and formats civil UTC time. QEMU's
# PL031 tracks the host wall clock, so the output must match YYYY-MM-DD HH:MM:SS
# UTC with a sane year (2025..2100). A wrong epoch->calendar algorithm would yield
# a garbage year, so the year range is a strong correctness check.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4646,server,nowait \
    -qmp tcp:127.0.0.1:4647,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/date.out"
python3 scripts/shell-demo.py 4646 4647 "${TMPDIR:-/tmp}/date.png" "date" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "UTC" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2}) UTC", s)
if not m:
    print("DATE TEST FAILED (no YYYY-MM-DD HH:MM:SS UTC line)"); sys.exit(1)
y, mo, d, hh, mm, ss = (int(x) for x in m.groups())
if not (2025 <= y <= 2100):
    print(f"DATE TEST FAILED (year {y} out of sane range -> calendar math wrong)"); sys.exit(1)
if not (1 <= mo <= 12 and 1 <= d <= 31 and hh <= 23 and mm <= 59 and ss <= 59):
    print(f"DATE TEST FAILED (field out of range: {m.group(0)})"); sys.exit(1)
print(f"DATE TEST PASSED  (RTC formatted as {m.group(0)}; civil UTC math correct)")
PY
