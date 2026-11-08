#!/usr/bin/env bash
# Verify the `ordinal` command formats a non-negative integer as its English
# ordinal string: last digit decides the suffix (1->st, 2->nd, 3->rd, else th)
# EXCEPT 11/12/13 which are always "th". The harness derives the expected suffix
# with the same rule in python, so it is self-checking. Also checks the usage line.
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
OUT="${TMPDIR:-/tmp}/ordinal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ordinal.png" \
    "ordinal 1" "ordinal 2" "ordinal 3" "ordinal 4" "ordinal 11" "ordinal 12" \
    "ordinal 13" "ordinal 21" "ordinal 101" "ordinal 111" "ordinal 113" "ordinal abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ordinal:|usage: ordinal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def suffix(n):
    if 11 <= (n % 100) <= 13:
        return "th"
    return {1: "st", 2: "nd", 3: "rd"}.get(n % 10, "th")

for n in [1, 2, 3, 4, 11, 12, 13, 21, 101, 111, 113]:
    want = f"ordinal: {n}{suffix(n)}"
    if want not in s:
        print(f"ORDINAL TEST FAILED (missing '{want}')"); sys.exit(1)

if "usage: ordinal <n>" not in s:
    print("ORDINAL TEST FAILED (missing usage line for non-numeric input)"); sys.exit(1)

print("ORDINAL TEST PASSED  (1st/2nd/3rd/4th, 11-13 th special-case, 21st/101st; bad->usage)")
PY
