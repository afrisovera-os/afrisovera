#!/usr/bin/env bash
# Verify hardware-enforced fault isolation (the microkernel guarantee): `faultdemo`
# spawns an EL0 process that writes to an unmapped address; the kernel must catch
# the fault, kill ONLY that process ("EL0 fault ... killing"), and keep running --
# proven by the shell still responding afterwards (echo FAULTSURVIVED prints).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4678,server,nowait \
    -qmp tcp:127.0.0.1:4679,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/fault.out"
python3 scripts/shell-demo.py 4678 4679 "${TMPDIR:-/tmp}/fault.png" \
    "faultdemo" "echo FAULTSURVIVED" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "EL0 fault|killing|FAULTSURVIVED" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
if "EL0 fault in P" not in s or "killing" not in s:
    print("FAULT TEST FAILED (kernel did not catch + kill the faulting EL0 process)"); sys.exit(1)
# the shell must still respond after the fault -> the OS survived
if "FAULTSURVIVED" not in s.split("killing", 1)[-1]:
    print("FAULT TEST FAILED (shell did not respond after the fault -- OS did not survive)"); sys.exit(1)
print("FAULT TEST PASSED  (EL0 process killed on a memory fault; the OS + shell survived)")
PY
