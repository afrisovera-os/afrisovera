#!/usr/bin/env bash
# Verify the base32 codec against RFC 4648: base32("foobar") = "MZXW6YTBOI======",
# and the decode round-trips back to "foobar". Standard-defined vectors, so a match
# proves correctness. Single capture, two commands.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4686,server,nowait \
    -qmp tcp:127.0.0.1:4687,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/base32.out"
python3 scripts/shell-demo.py 4686 4687 "${TMPDIR:-/tmp}/base32.png" \
    "base32 foobar" "base32 -d MZXW6YTBOI======" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "\[base32\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
if "[base32] MZXW6YTBOI======" not in s:
    print("BASE32 TEST FAILED (encode of 'foobar' != MZXW6YTBOI======)"); sys.exit(1)
if "[base32] foobar" not in s:
    print("BASE32 TEST FAILED (decode of MZXW6YTBOI====== != foobar)"); sys.exit(1)
print("BASE32 TEST PASSED  (RFC 4648: foobar <-> MZXW6YTBOI======)")
PY
