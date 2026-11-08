#!/usr/bin/env bash
# Verify the Asili assistant's NEW core-concept knowledge-base entries (docs/08+10): "what is a
# capability"->unforgeable token, "what is a sandbox"->sandboxed WASM space, "what is sideloading"->
# a right, "what is the bootloader"->unlockable. On-device, private.
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
OUT="${TMPDIR:-/tmp}/askconcept.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askconcept.png" \
    "echo warmup" "ask what is a capability" "ask what is a sandbox" \
    "ask what is sideloading" "ask what is the bootloader" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# each concept query must return its dedicated knowledge-base answer
assert "[asili] A CAPABILITY IS AN UNFORGEABLE TOKEN" in s
assert "[asili] EVERY APP RUNS SANDBOXED" in s
assert "[asili] SIDELOADING IS A RIGHT" in s
assert "[asili] THE BOOTLOADER IS UNLOCKABLE" in s

print("ASKCONCEPT TEST PASSED  (capability->unforgeable token, sandbox->WASM space, sideloading->a "
      "right, bootloader->unlockable; on-device Asili concept knowledge base)")
PY
