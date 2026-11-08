#!/usr/bin/env bash
# Verify the `fwcfg` command reads the QEMU fw_cfg control device over MMIO. The
# signature key (0x0000) deterministically returns ASCII "QEMU"; the file-count key
# (0x0019) returns a small positive number (varies with qemu version/devices, so we
# only assert >= 1). id is the interface bitmap (0x3 = traditional + DMA on virt).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4790,server,nowait \
    -qmp tcp:127.0.0.1:4791,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/fwcfg.out"
python3 scripts/shell-demo.py 4790 4791 "${TMPDIR:-/tmp}/fwcfg.png" "fwcfg" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "fwcfg:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "fwcfg: signature = QEMU" not in s:
    print("FWCFG TEST FAILED (signature != QEMU)"); sys.exit(1)

m = re.search(r"fwcfg: file count = (\d+)", s)
if not m:
    print("FWCFG TEST FAILED (missing file-count line)"); sys.exit(1)
count = int(m.group(1))
if count < 1:
    print(f"FWCFG TEST FAILED (file count {count} < 1)"); sys.exit(1)

if "fwcfg: id = 0x" not in s:
    print("FWCFG TEST FAILED (missing id line)"); sys.exit(1)

print(f"FWCFG TEST PASSED  (signature QEMU; file count {count} >= 1; id present)")
PY
