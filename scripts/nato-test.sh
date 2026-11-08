#!/usr/bin/env bash
# Verify the `nato` command spells text in the NATO phonetic alphabet: SOS ->
# Sierra Oscar Sierra; ABC -> Alpha Bravo Charlie; Pi3 -> Papa India Three
# (case-folded letters + digit); words space-separated, punctuation/space ignored.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4802,server,nowait \
    -qmp tcp:127.0.0.1:4803,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/nato.out"
python3 scripts/shell-demo.py 4802 4803 "${TMPDIR:-/tmp}/nato.png" \
    "nato SOS" "nato ABC" "nato Pi3" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nato:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "nato: Sierra Oscar Sierra",
    "nato: Alpha Bravo Charlie",
    "nato: Papa India Three",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"NATO TEST FAILED (missing: {missing})"); sys.exit(1)
print("NATO TEST PASSED  (SOS/ABC spelled; case-folded letters + digit in Pi3)")
PY
