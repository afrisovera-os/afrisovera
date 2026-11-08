#!/usr/bin/env bash
# Verify the Asili assistant's NEW even/odd capability (docs/09 AI pillar): "ask is 42 even or odd"->
# "42 is EVEN", "ask is 13 even or odd"->"13 is ODD", "ask is 0 even"->"0 is EVEN", "ask is 7 odd"->
# "7 is ODD", "ask is 100 even"->"100 is EVEN". Parity of the first integer, guarded on even/odd,
# dispatched before try_math (no operator). Self-checking: python n % 2.
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
OUT="${TMPDIR:-/tmp}/askevenodd.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askevenodd.png" \
    "echo warmup" "ask is 42 even or odd" "ask is 13 even or odd" "ask is 0 even" \
    "ask is 7 odd" "ask is 100 even" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

for n in (42, 13, 0, 7, 100):
    kind = "EVEN" if n % 2 == 0 else "ODD"
    want = f"[asili] {n} is {kind} (CHECKED ON-DEVICE)"
    if want not in s:
        print(f"ASKEVENODD TEST FAILED (missing '{want}')"); sys.exit(1)

print("ASKEVENODD TEST PASSED  (42->EVEN, 13->ODD, 0->EVEN, 7->ODD, 100->EVEN; on-device Asili "
      "even/odd, dispatched before math)")
PY
