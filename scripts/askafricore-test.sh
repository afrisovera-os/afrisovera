#!/usr/bin/env bash
# Verify the Asili assistant's NEW AfriCore sovereign-service knowledge-base entries (docs/09 AI
# pillar): "what is afriid"->identity, "afripay"->payment rails, "afripush"->notifications,
# "afrishare"->P2P sharing, "afristore"->federated app store. The afristore branch is checked
# BEFORE the generic "store" (signing) branch. On-device, private.
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
OUT="${TMPDIR:-/tmp}/askafricore.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askafricore.png" \
    "echo warmup" "ask what is afriid" "ask what is afripay" "ask what is afripush" \
    "ask what is afrishare" "ask what is afristore" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# each AfriCore service query must return its dedicated knowledge-base answer
assert "[asili] AFRIID IS THE SOVEREIGN IDENTITY SERVICE" in s
assert "[asili] AFRIPAY IS THE SOVEREIGN PAYMENT RAILS" in s
assert "[asili] AFRIPUSH IS THE NOTIFICATION SERVICE" in s
assert "[asili] AFRISHARE IS THE PEER-TO-PEER SHARING SERVICE" in s
assert "[asili] AFRISTORE IS THE FEDERATED APP STORE" in s   # matched BEFORE generic "store"

print("ASKAFRICORE TEST PASSED  (afriid->identity, afripay->rails, afripush->notifications, "
      "afrishare->P2P, afristore->federated store; on-device Asili AfriCore knowledge base)")
PY
