#!/usr/bin/env bash
# Verify the `squeeze` command collapses consecutive runs of a char to one
# (like `tr -s`), squeezing ONLY the given char (others untouched). Deterministic:
#   squeeze o foootball -> "fotball"  (f-ooo-tball: the ooo run -> a single o)
#   squeeze l balloon   -> "baloon"   (ba-ll-oon: ll -> l; the oo is NOT squeezed)
#   squeeze x abcabc    -> "abcabc"   (no x, so no run to collapse)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4750,server,nowait \
    -qmp tcp:127.0.0.1:4751,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/squeeze.out"
python3 scripts/shell-demo.py 4750 4751 "${TMPDIR:-/tmp}/squeeze.png" "squeeze o foootball" "squeeze l balloon" "squeeze x abcabc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "squeeze: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["squeeze: fotball", "squeeze: baloon", "squeeze: abcabc"]
missing = [w for w in want if w not in s]
if missing:
    print(f"SQUEEZE TEST FAILED (missing: {missing})"); sys.exit(1)
print("SQUEEZE TEST PASSED  (foootball->fotball; balloon->baloon; abcabc unchanged)")
PY
