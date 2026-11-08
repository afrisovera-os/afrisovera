#!/usr/bin/env bash
# Verify `nextdodecagonal <n>` reports the SMALLEST dodecagonal Dod(k)=k(5k-4) (k>=1:
# 1,12,33,64,105,156,...) that is >= n, with index k -- the 12-gonal member of the next* family,
# pairing with the dodecagonal membership test. Tests 2->Dod(2)=12, 12->Dod(2)=12, 13->Dod(3)=33,
# 70->Dod(5)=105, 0->Dod(1)=1. Self-checking: python derives the smallest dodecagonal >= n.
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
OUT="${TMPDIR:-/tmp}/nextdodecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextdodecagonal.png" \
    "echo warmup" "nextdodecagonal 2" "nextdodecagonal 13" "nextdodecagonal 70" \
    "nextdodecagonal 0" "nextdodecagonal 12" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextdodecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dodec(k): return k * (5 * k - 4)

def nextdodec(n):                    # DERIVED reference: smallest Dod(k) >= n, k>=1
    k = 1
    while dodec(k) < n: k += 1
    return k

for n in (2, 13, 70, 0, 12):
    k = nextdodec(n)
    want = f"nextdodecagonal: smallest dodecagonal >= {n} is Dod({k}) = {dodec(k)}"
    if want not in s:
        print(f"NEXTDODECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

assert nextdodec(2) == 2 and dodec(2) == 12
assert nextdodec(13) == 3 and dodec(3) == 33
assert nextdodec(70) == 5 and dodec(5) == 105
assert nextdodec(0) == 1 and dodec(1) == 1
assert nextdodec(12) == 2
print("NEXTDODECAGONAL TEST PASSED  (2/13/70/0/12 vs python smallest-Dod>=n)")
PY
