#!/usr/bin/env bash
# Verify the `isbouncy <n>` command classifies bouncy numbers: digits neither non-
# decreasing (increasing, e.g. 134468) nor non-increasing (decreasing, e.g. 66420).
# The smallest bouncy number is 101; 155349 is bouncy. Distinct from isdigitsorted
# (non-decreasing only). Self-checking: python replicates the exact three-branch msg.
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
OUT="${TMPDIR:-/tmp}/isbouncy.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 101 (smallest bouncy), 134468 (non-decreasing), 66420 (non-increasing), 155349 (bouncy).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isbouncy.png" \
    "echo warmup" "isbouncy 101" "isbouncy 134468" "isbouncy 66420" "isbouncy 155349" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isbouncy:|usage: isbouncy" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):
    d = str(n)
    nondec = all(d[i] <= d[i + 1] for i in range(len(d) - 1))
    noninc = all(d[i] >= d[i + 1] for i in range(len(d) - 1))
    if not nondec and not noninc:
        return f"isbouncy: {n} IS bouncy"
    if nondec:
        return f"isbouncy: {n} is NOT bouncy (digits non-decreasing)"
    return f"isbouncy: {n} is NOT bouncy (digits non-increasing)"

for n in [101, 134468, 66420, 155349]:
    want = message(n)
    if want not in s:
        print(f"ISBOUNCY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "isbouncy: 101 IS bouncy" in s                                   # smallest bouncy
assert "isbouncy: 134468 is NOT bouncy (digits non-decreasing)" in s    # increasing
assert "isbouncy: 66420 is NOT bouncy (digits non-increasing)" in s     # decreasing
assert "isbouncy: 155349 IS bouncy" in s

print("ISBOUNCY TEST PASSED  (101 IS, 134468 non-decreasing, 66420 non-increasing, 155349 IS "
      "vs python; digits neither up- nor down-sorted)")
PY
