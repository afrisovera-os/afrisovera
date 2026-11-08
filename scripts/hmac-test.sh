#!/usr/bin/env bash
# Verify HMAC-SHA256 against RFC 4231 Test Case 2: key="Jefe", message="what do ya
# want for nothing?" -> HMAC-SHA256 = 5bdcc146...ec3843. Writing that message and
# running `hmac Jefe <file>` must produce exactly that digest -- a standard-defined
# value, so a match proves the implementation is correct.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
EXPECT="5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4682,server,nowait \
    -qmp tcp:127.0.0.1:4683,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/hmac.out"
python3 scripts/shell-demo.py 4682 4683 "${TMPDIR:-/tmp}/hmac.png" \
    "write jefe.txt what do ya want for nothing?" "hmac Jefe jefe.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "hmac-sha256 jefe" "$OUT" | sed 's/^/  /'

if grep -aiq "$EXPECT" "$OUT"; then
    echo "HMAC TEST PASSED  (RFC 4231 TC2: HMAC-SHA256(Jefe, ...) = ${EXPECT:0:16}...)"
else
    echo "HMAC TEST FAILED (expected RFC 4231 TC2 digest $EXPECT)"; exit 1
fi
