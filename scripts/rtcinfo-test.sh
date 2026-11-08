#!/usr/bin/env bash
# Verify the `rtcinfo` command reads the PL031 RTC's PrimeCell identity + live
# data register over MMIO. The identity is fixed in silicon (deterministic):
# part 0x031 (PL031), designer 0x41 (ARM), PrimeCell magic 0xb105f00d. RTCDR is
# the live clock (a Unix epoch), so we assert it is present and > 0, not an
# exact value.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4770,server,nowait \
    -qmp tcp:127.0.0.1:4771,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/rtcinfo.out"
python3 scripts/shell-demo.py 4770 4771 "${TMPDIR:-/tmp}/rtcinfo.png" "rtcinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rtcinfo: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["part 0x031 (PL031)", "designer 0x41 (ARM)", "PrimeCell ID 0xb105f00d"]
missing = [w for w in want if w not in s]
if missing:
    print(f"RTCINFO TEST FAILED (missing identity: {missing})"); sys.exit(1)
m = re.search(r"RTCDR \(data\) = (\d+)", s)
if not m or int(m.group(1)) == 0:
    print("RTCINFO TEST FAILED (no live RTCDR value)"); sys.exit(1)
print(f"RTCINFO TEST PASSED  (PL031 part 0x031/ARM/magic 0xb105f00d; live RTCDR={m.group(1)})")
PY
