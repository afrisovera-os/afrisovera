#!/usr/bin/env bash
# Prove the user-facing `hash` command computes real, correct digests: write a
# file with known content ("abc") into the RAM filesystem, hash it, and assert
# the kernel's SHA-256 and SHA-512 match the reference digests from Python's
# hashlib. No disk needed (RAM-only). Needs qemu + python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

REF256="$(python3 -c "import hashlib; print(hashlib.sha256(b'abc').hexdigest())")"
REF512="$(python3 -c "import hashlib; print(hashlib.sha512(b'abc').hexdigest())")"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4570,server,nowait \
    -qmp tcp:127.0.0.1:4571,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4570 4571 "${TMPDIR:-/tmp}/hash-test.png" "write hashme abc" "hash hashme" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

echo "$SER" | grep -iE "\[hash\]|sha256|sha512" | sed 's/^/  /'
echo "  reference sha256(abc) = $REF256"

echo "$SER" | grep -q "sha256 $REF256" \
    || { echo "HASH TEST FAILED (SHA-256 does not match hashlib)"; exit 1; }
echo "$SER" | grep -q "sha512 $REF512" \
    || { echo "HASH TEST FAILED (SHA-512 does not match hashlib)"; exit 1; }
echo "HASH TEST PASSED  (kernel SHA-256 + SHA-512 of \"abc\" match Python hashlib)"
