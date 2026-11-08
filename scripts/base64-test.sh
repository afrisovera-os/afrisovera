#!/usr/bin/env bash
# Verify base64 encode/decode: `base64 HELLO` -> SEVMTE8= and `base64 -d SEVMTE8=`
# -> HELLO (a round trip). RAM-only.
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
OUT="${TMPDIR:-/tmp}/base64.out"
python3 scripts/shell-demo.py 4680 4681 "${TMPDIR:-/tmp}/base64.png" \
    "base64 HELLO" "base64 -d SEVMTE8=" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "base64" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
ok_enc = "[base64] SEVMTE8=" in s
# the decode output "[base64] HELLO" must appear after the decode command
after_dec = s.split("base64 -d SEVMTE8=", 1)[-1]
ok_dec = "[base64] HELLO" in after_dec
if ok_enc and ok_dec:
    print("BASE64 TEST PASSED  (HELLO -> SEVMTE8= -> HELLO round trip)")
    sys.exit(0)
print(f"BASE64 TEST FAILED (encode={ok_enc} decode={ok_dec})")
sys.exit(1)
PY
