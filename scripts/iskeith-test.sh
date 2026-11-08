#!/usr/bin/env bash
# Verify the `iskeith <n>` command classifies Keith (repfigit) numbers: seed a
# sequence with n's own decimal digits, then repeatedly append the sum of the
# last d terms (d = digit count); n IS a Keith number if the sequence lands
# exactly on n (14: 1,4,5,9,14; 197: 1,9,7,17,33,57,107,197). Single-digit n is
# NOT Keith by convention. Distinct from isfib (membership in the GLOBAL
# Fibonacci sequence). Self-checking: python runs the same digit-seeded sequence.
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
OUT="${TMPDIR:-/tmp}/iskeith.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iskeith.png" \
    "echo warmup" "iskeith 14" "iskeith 742" "iskeith 13" "iskeith 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iskeith:|usage: iskeith" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def keith_seq(n):
    ds = [int(c) for c in str(n)]; d = len(ds)
    seq = ds[:]
    while seq[-1] < n:
        seq.append(sum(seq[-d:]))
    return seq[-1]  # first term >= n

# IS cases
for n in [14, 742]:
    assert keith_seq(n) == n, f"python: {n} should be Keith"
    want = f"iskeith: {n} IS a Keith number (repfigit sequence lands on it)"
    if want not in s:
        print(f"ISKEITH TEST FAILED (missing '{want}')"); sys.exit(1)

# NOT via overshoot: 13 -> 1,3,4,7,11,18 -> overshoot to 18
over = keith_seq(13)
assert over == 18, f"python: 13 overshoots to {over}"
want = f"iskeith: 13 is NOT a Keith number (sequence overshot to {over})"
if want not in s:
    print(f"ISKEITH TEST FAILED (missing '{want}')"); sys.exit(1)

# NOT via single digit
if "iskeith: 7 is NOT a Keith number (needs >= 2 digits)" not in s:
    print("ISKEITH TEST FAILED (single-digit case)"); sys.exit(1)

print("ISKEITH TEST PASSED  (14/742 IS, 13 overshoots to 18 NOT, 7 single-digit "
      "NOT; digit-seeded repfigit sequence vs python, distinct from isfib)")
PY
