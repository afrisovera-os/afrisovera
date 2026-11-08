#!/usr/bin/env bash
# Verify the `uartflags` command reads the PL011 UART control/line-control/flag
# registers over MMIO (the config/live complement to uartinfo). On QEMU virt the
# CONTROL + LINE-CONTROL registers are deterministic: UARTCR = 0x00000300
# (UARTEN off, TXE/RXE on -- QEMU's PL011 transmits without UARTEN), UARTLCR_H =
# 0x00000010 (WLEN=5 bits, FEN on). UARTFR is the live flag register (kept structural).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4854,server,nowait \
    -qmp tcp:127.0.0.1:4855,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/uartflags.out"
python3 scripts/shell-demo.py 4854 4855 "${TMPDIR:-/tmp}/uartflags.png" "uartflags" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uartflags: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

# Deterministic config registers on QEMU virt.
want = [
    "uartflags: UARTCR = 0x00000300 (UARTEN=off TXE=on RXE=on)",
    "uartflags: UARTLCR_H = 0x00000010 (WLEN=5 bits  FEN=yes)",
]
for w in want:
    if w not in s:
        print(f"UARTFLAGS TEST FAILED (expected '{w}')"); sys.exit(1)

# Live flag register: line present + well-formed (values may vary with FIFO state).
if not re.search(r"uartflags: UARTFR = 0x[0-9a-f]{8} \(TXFE=(yes|no) RXFE=(yes|no) BUSY=(yes|no)\)", s):
    print("UARTFLAGS TEST FAILED (malformed/missing UARTFR line)"); sys.exit(1)

print("UARTFLAGS TEST PASSED  (UARTCR/UARTLCR_H pinned; UARTFR flag line well-formed)")
PY
