#!/usr/bin/env bash
# Verify the `pmap` memory-map command. Asserts the single-line summary (robust to
# truncation): kernel@0x40000000, a positive physical-frame count, and the MMIO
# device count -- plus that the shell survives afterwards (uptime prints).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4680,server,nowait \
    -qmp tcp:127.0.0.1:4681,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/pmap.out"
python3 scripts/shell-demo.py 4680 4681 "${TMPDIR:-/tmp}/pmap.png" "pmap" "uptime" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "pmap: kernel@|UART \(PL011\)" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
# The summary carries the arena's growth since 2.740 ("heap U/A (grown N B)"), which
# this assertion was written before and went on demanding the older, shorter form --
# a stale assertion, red for a line the kernel prints correctly. Growth is optional
# here so the suite reds on the CONTENT it checks, not on the next field appended.
m = re.search(
    r"pmap: kernel@0x40000000, heap \d+/\d+(?: \(grown \d+ B\))?, (\d+) frames, "
    r"wasm 64KiB, (\d+) MMIO devices", s)
if not m:
    print("PMAP TEST FAILED (no pmap summary line in the expected form)"); sys.exit(1)
frames, ndev = int(m.group(1)), int(m.group(2))
if frames <= 0:
    print(f"PMAP TEST FAILED (frame count {frames} not positive)"); sys.exit(1)
if ndev < 1:
    print(f"PMAP TEST FAILED (MMIO device count {ndev} < 1)"); sys.exit(1)
# shell must still respond after the multi-line pmap block
if "uptime" not in s.split("MMIO devices", 1)[-1]:
    print("PMAP TEST FAILED (shell did not respond after pmap)"); sys.exit(1)
print(f"PMAP TEST PASSED  (kernel@0x40000000, {frames} frames, {ndev} MMIO devices; shell alive)")
PY
