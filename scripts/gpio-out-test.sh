#!/usr/bin/env bash
# Verify GPIO output: `gpio set 0 1` sets pin 0 as an output and drives it high;
# the PL061 data-register readback must then show bit 0 set (0x01) and the
# direction register bit 0 set. This proves the driver drives a real hardware
# output pin and reads it back over MMIO (QEMU's PL061 reflects output levels).
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
OUT="${TMPDIR:-/tmp}/gpio-out.out"
python3 scripts/shell-demo.py 4660 4661 "${TMPDIR:-/tmp}/gpio-out.png" "gpio set 0 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "gpio: pin0" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"gpio: pin0 set to 1, readback (0x[0-9a-fA-F]+) \(dir (0x[0-9a-fA-F]+)\)", s)
if not m:
    print("GPIO-OUT TEST FAILED (no gpio set output line)"); sys.exit(1)
readback = int(m.group(1), 16); direction = int(m.group(2), 16)
if not (direction & 0x01):
    print(f"GPIO-OUT TEST FAILED (pin0 not set as output; dir={hex(direction)})"); sys.exit(1)
if not (readback & 0x01):
    print(f"GPIO-OUT TEST FAILED (pin0 output not reflected in readback {hex(readback)})"); sys.exit(1)
print(f"GPIO-OUT TEST PASSED  (pin0 driven high: readback {hex(readback)}, dir {hex(direction)})")
PY
