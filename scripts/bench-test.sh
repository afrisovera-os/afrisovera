#!/usr/bin/env bash
# Verify the `bench` command runs an on-device SHA-256 micro-benchmark timed by the
# hi-res physical counter. Asserts the deterministic parts (512 iters x 4096 bytes)
# and that the measured elapsed time is a positive integer (proving the CNTPCT
# timing actually ran). The MiB/s figure is informational.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4674,server,nowait \
    -qmp tcp:127.0.0.1:4675,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/bench.out"
python3 scripts/shell-demo.py 4674 4675 "${TMPDIR:-/tmp}/bench.png" "bench" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "bench: sha256" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"bench: sha256 512 iters x 4096 bytes = 2048 KiB in (\d+) us \((\d+) MiB/s\)", s)
if not m:
    print("BENCH TEST FAILED (no bench line with the expected iters/bytes)"); sys.exit(1)
us = int(m.group(1))
if us <= 0:
    print(f"BENCH TEST FAILED (elapsed {us} us -- the hi-res timer did not advance)"); sys.exit(1)
print(f"BENCH TEST PASSED  (sha256 512x4096B in {us} us, {m.group(2)} MiB/s, timed by CNTPCT)")
PY
