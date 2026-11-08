#!/usr/bin/env bash
# Verify the `timerinfo` command reports the ARM generic timer and proves the
# counter advances. CNTFRQ is a stable positive value on QEMU virt; the exact
# measured microseconds for a 10ms busywait vary with host jitter, so we assert
# it lands in a sane window (proves the counter genuinely advanced, not a stub).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4704,server,nowait \
    -qmp tcp:127.0.0.1:4705,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/timerinfo.out"
python3 scripts/shell-demo.py 4704 4705 "${TMPDIR:-/tmp}/timerinfo.png" "timerinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "timerinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
mfreq = re.search(r"CNTFRQ = (\d+) Hz", s)
if not mfreq or int(mfreq.group(1)) <= 0:
    print("TIMERINFO TEST FAILED (no positive CNTFRQ reported)"); sys.exit(1)
mus = re.search(r"10ms busywait measured (\d+) us", s)
if not mus:
    print("TIMERINFO TEST FAILED (no 10ms busywait measurement line)"); sys.exit(1)
us = int(mus.group(1))
if not (3000 <= us <= 40000):
    print(f"TIMERINFO TEST FAILED (10ms busywait measured {us} us -- outside [3000,40000])"); sys.exit(1)
print(f"TIMERINFO TEST PASSED  (CNTFRQ={mfreq.group(1)} Hz; 10ms busywait -> {us} us, counter advancing)")
PY
