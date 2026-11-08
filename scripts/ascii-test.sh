#!/usr/bin/env bash
# Verify the `ascii` command maps a code point <-> its printable-ASCII glyph:
#   ascii 65  -> ascii: 65 = 'A'  0x41  0o101   (number -> glyph)
#   ascii A   -> ascii: 'A' = 65  0x41  0o101   (glyph -> code point)
#   ascii 32  -> ascii: 32 = ' '  0x20  0o040   (space is printable)
#   ascii 10  -> ascii: 10 = LF  0x0a (non-printable)   (control-char name)
#   ascii 127 -> ascii: 127 = DEL  0x7f (non-printable)
#   ascii 200 / ascii (empty) -> usage line
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
OUT="${TMPDIR:-/tmp}/ascii.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ascii.png" \
    "ascii 65" "ascii A" "ascii 32" "ascii 10" "ascii 127" "ascii 200" "ascii" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ascii:|usage: ascii" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

checks = [
    "ascii: 65 = 'A'  0x41  0o101",
    "ascii: 'A' = 65  0x41  0o101",
    "ascii: 32 = ' '  0x20  0o040",
    "ascii: 10 = LF  0x0a (non-printable)",
    "ascii: 127 = DEL  0x7f (non-printable)",
    "usage: ascii <0-127 | single-char>",
]
for want in checks:
    if want not in s:
        print(f"ASCII TEST FAILED (missing '{want}')"); sys.exit(1)

print("ASCII TEST PASSED  (65<->A, space printable, LF/DEL named, out-of-range/empty -> usage)")
PY
