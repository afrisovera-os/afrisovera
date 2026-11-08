#!/usr/bin/env bash
# Verify interrupt-driven UART RX: type a command (echo HELLO) and then sysinfo.
# Assert (a) the shell still echoes HELLO cleanly (no reorder/duplication from a
# ring/poll race) and (b) sysinfo reports "uart rx: N irqs" with N > 0 -- proving
# the typed bytes arrived via the PL011 RX interrupt -> ring, not a hardware poll.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4640,server,nowait \
    -qmp tcp:127.0.0.1:4641,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/uart-irq.out"
# `irqstat` prints a single line (robust to capture truncation under load); the
# echo proves the shell still receives cleanly, irqstat proves it's IRQ-driven.
python3 scripts/shell-demo.py 4640 4641 "${TMPDIR:-/tmp}/uart-irq.png" \
    "echo HELLO" "irqstat" "uptime" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "^HELLO|uart rx" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
# (a) clean echo — HELLO must appear on its own line (no reorder/dup garble)
if not re.search(r"(^|\n)HELLO\b", s):
    print("UART-IRQ TEST FAILED (echo did not return a clean HELLO)"); sys.exit(1)
# (b) interrupt-driven: irqstat reports a non-zero RX irq count
m = re.search(r"uart rx (\d+)", s)
if not m:
    print("UART-IRQ TEST FAILED (irqstat did not report the uart rx irq count)"); sys.exit(1)
n = int(m.group(1))
if n <= 0:
    print(f"UART-IRQ TEST FAILED (rx irq count is {n}; input not interrupt-driven)"); sys.exit(1)
print(f"UART-IRQ TEST PASSED  (clean echo HELLO; {n} PL011 RX interrupts serviced -> input is interrupt-driven)")
PY
