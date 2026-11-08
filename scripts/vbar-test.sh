#!/usr/bin/env bash
# Verify the `vbar` command reads VBAR_EL1 (the Vector Base Address Register): the
# base address of the EL1 exception-vector table + its 2KB-alignment check. Distinct
# from mmuinfo/tcrinfo/ttbr. On QEMU virt the kernel installs a vector table at a
# NON-ZERO RAM address (0x4xxxxxxx) that is 2KB-aligned. The exact address drifts as
# the kernel image grows, so we assert STRUCTURE (non-zero 0x4... base, aligned=yes).
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
OUT="${TMPDIR:-/tmp}/vbar.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/vbar.png" "vbar" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vbar:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "vbar: VBAR_EL1 = 0x" not in s:
    print("VBAR TEST FAILED (missing raw VBAR_EL1 line)"); sys.exit(1)

# Structural: the exception-vector base is a non-zero RAM address (0x4xxxxxxx).
# The exact value drifts as the kernel image grows, so match the range, not a literal.
m = re.search(r"vbar: exception vector base = 0x4([0-9a-f]+)", s)
if not m:
    print("VBAR TEST FAILED (vector base not a non-zero 0x4... RAM address)"); sys.exit(1)

# Deterministic fact: the architecture requires VBAR to be 2KB-aligned.
if "vbar: 2KB-aligned = yes" not in s:
    print("VBAR TEST FAILED (vector base not reported 2KB-aligned)"); sys.exit(1)

print("VBAR TEST PASSED  (VBAR_EL1 = non-zero 0x4... RAM address, 2KB-aligned)")
PY
