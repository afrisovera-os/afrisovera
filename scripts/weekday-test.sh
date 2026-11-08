#!/usr/bin/env bash
# Verify the `weekday` command computes the Gregorian day-of-week via Zeller's
# congruence (a pure compute, no RTC read; distinct from `date`). The harness
# derives the EXPECTED day name from Python's datetime for each vector, so it is
# self-checking: weekday 2000 1 1 -> Saturday, 2026 8 7 -> Friday,
# 1970 1 1 -> Thursday, 2024 2 29 -> Thursday (leap day); a bad month -> usage.
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
OUT="${TMPDIR:-/tmp}/weekday.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/weekday.png" \
    "weekday 2000 1 1" "weekday 2026 8 7" "weekday 1970 1 1" "weekday 2024 2 29" \
    "weekday 2020 13 1" "weekday" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "weekday:|usage: weekday" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, datetime
s = open(sys.argv[1], "rb").read().decode("latin1")

vectors = [(2000, 1, 1), (2026, 8, 7), (1970, 1, 1), (2024, 2, 29)]
for y, m, d in vectors:
    name = datetime.date(y, m, d).strftime("%A")
    want = f"weekday: {y}-{m:02d}-{d:02d} is {name}"
    if want not in s:
        print(f"WEEKDAY TEST FAILED (missing '{want}')"); sys.exit(1)

if "usage: weekday <year> <month> <day>" not in s:
    print("WEEKDAY TEST FAILED (missing usage line for bad input)"); sys.exit(1)

print("WEEKDAY TEST PASSED  (Zeller vs Python datetime: 2000-01-01 Sat, 2026-08-07 Fri, 1970-01-01 Thu, 2024-02-29 Thu; bad->usage)")
PY
