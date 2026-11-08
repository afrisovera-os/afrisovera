#!/usr/bin/env bash
# Verify the Asili assistant's NEW knowledge-base entries for the OS components (docs/09 AI
# pillar): "what is nia" -> WASM app runtime, "tell me about daraja" -> foreign-app bridge,
# "what is sankofa" -> UI framework, "what is tonga" -> compositor, "what is mansa" ->
# microkernel. On-device, private.
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
OUT="${TMPDIR:-/tmp}/askkb.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askkb.png" \
    "echo warmup" "ask what is nia" "ask tell me about daraja" "ask what is sankofa" \
    "ask what is tonga" "ask what is mansa" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# each component query must return its dedicated knowledge-base answer
assert "[asili] NIA IS THE WASM APP RUNTIME" in s
assert "[asili] DARAJA IS THE FOREIGN-APP BRIDGE" in s
assert "[asili] SANKOFA UI IS THE DECLARATIVE" in s
assert "[asili] TONGA IS THE DISPLAY COMPOSITOR" in s
assert "[asili] MANSA IS THE MICROKERNEL" in s

print("ASKKB TEST PASSED  (nia->WASM runtime, daraja->bridge, sankofa->UI, tonga->compositor, "
      "mansa->microkernel; on-device Asili knowledge base)")
PY
