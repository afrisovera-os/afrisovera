#!/usr/bin/env bash
# Verify the PL061 GPIO driver: the `gpio` command reads the controller's ARM
# PrimeCell identification registers over MMIO and must report part 0x061 (the
# PL061 part number). A correct 0x061 proves a real MMIO read to a distinct
# peripheral (not a stubbed value). Single-line assertion.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4656,server,nowait \
    -qmp tcp:127.0.0.1:4657,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/gpio.out"
python3 scripts/shell-demo.py 4656 4657 "${TMPDIR:-/tmp}/gpio.png" "gpio" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "gpio: PL061|not detected" "$OUT" | sed 's/^/  /'

grep -aq "part 0x061" "$OUT" \
    && echo "GPIO TEST PASSED  (PL061 identified via PrimeCell part 0x061 over MMIO)" \
    || { echo "GPIO TEST FAILED (expected PL061 part 0x061)"; exit 1; }
