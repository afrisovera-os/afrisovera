#!/usr/bin/env bash
# Verify the `uuid` command produces RFC 4122 version-4 UUIDs from the CSPRNG. The
# format is deterministic (version nibble '4', variant nibble [89ab]) even though
# the value is random, so the regex proves correctness; two calls must also differ
# (proving it draws from the RNG, not a stub).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4692,server,nowait \
    -qmp tcp:127.0.0.1:4693,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/uuid.out"
python3 scripts/shell-demo.py 4692 4693 "${TMPDIR:-/tmp}/uuid.png" "uuid" "uuid" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "uuid:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
pat = re.compile(r"uuid: ([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})")
ids = pat.findall(s)
if len(ids) < 2:
    print(f"UUID TEST FAILED (found {len(ids)} valid v4 UUIDs; need 2)"); sys.exit(1)
if ids[0] == ids[1]:
    print(f"UUID TEST FAILED (two calls returned the same UUID {ids[0]} -- not random)"); sys.exit(1)
print(f"UUID TEST PASSED  (two distinct RFC 4122 v4 UUIDs: {ids[0]}, {ids[1]})")
PY
