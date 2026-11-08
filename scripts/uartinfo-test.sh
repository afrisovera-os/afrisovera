#!/usr/bin/env bash
# Verify the `uartinfo` command reads the PL011 UART's PrimeCell identification
# registers over MMIO. These are fixed in silicon, so the decode is fully
# deterministic: part 0x011 (PL011), designer 0x41 (ARM), and the standard
# PrimeCell magic 0xb105f00d -- the magic is a strong proof the ID registers
# were read (and assembled) correctly.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4746,server,nowait \
    -qmp tcp:127.0.0.1:4747,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/uartinfo.out"
python3 scripts/shell-demo.py 4746 4747 "${TMPDIR:-/tmp}/uartinfo.png" "uartinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uartinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["part 0x011 (PL011)", "designer 0x41 (ARM)", "PrimeCell ID 0xb105f00d"]
missing = [w for w in want if w not in s]
if missing:
    print(f"UARTINFO TEST FAILED (missing: {missing})"); sys.exit(1)
print("UARTINFO TEST PASSED  (PL011 part 0x011, ARM 0x41, PrimeCell magic 0xb105f00d)")
PY
