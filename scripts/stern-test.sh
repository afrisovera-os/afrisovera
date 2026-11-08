#!/usr/bin/env bash
# Verify `stern <n>` reports Stern's diatomic (fusc) sequence a(n): a(0)=0, a(1)=1, a(2n)=a(n),
# a(2n+1)=a(n)+a(n+1). First terms 0,1,1,2,1,3,2,3,1,4,3,5,2. Tests n=0,1,9,11,12,90. Self-checking:
# python recursive fusc.
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
OUT="${TMPDIR:-/tmp}/stern.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stern.png" \
    "echo warmup" "stern 0" "stern 9" "stern 11" "stern 12" "stern 90" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stern:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
sys.setrecursionlimit(10000)
s = open(sys.argv[1], "rb").read().decode("latin1")

from functools import lru_cache
@lru_cache(maxsize=None)
def fusc(n):                         # DERIVED reference: the defining recurrence
    if n == 0: return 0
    if n == 1: return 1
    return fusc(n // 2) if n % 2 == 0 else fusc(n // 2) + fusc(n // 2 + 1)

for n in (0, 9, 11, 12, 90):
    want = f"stern: a({n}) = {fusc(n)}"
    if want not in s:
        print(f"STERN TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the opening terms
assert [fusc(n) for n in range(13)] == [0, 1, 1, 2, 1, 3, 2, 3, 1, 4, 3, 5, 2]
print("STERN TEST PASSED  (a(0)=0, a(9)=4, a(11)=5, a(12)=2, a(90) vs python fusc)")
PY
