#!/usr/bin/env bash
# Verify `rotl <n> <k>` rotates the 64 bits of n LEFT by k (bits off the top wrap to the bottom):
# rotl 1 1 -> 2, rotl 1 63 -> 2^63, rotl 2^63 1 -> 1, rotl 255 8 -> 65280. k taken mod 64. The
# barrel-rotate used in hashing; distinct from bitrev (reverse) and a plain shift. Self-checking:
# python replicates a u64 rotate_left for every reference (value + zero-padded hex).
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
OUT="${TMPDIR:-/tmp}/rotl.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotl.png" \
    "echo warmup" "rotl 1 1" "rotl 1 63" "rotl 9223372036854775808 1" "rotl 255 8" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotl:|usage: rotl" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
M = (1 << 64) - 1

def rotl(n, k):  # mirror u64::rotate_left: k mod 64, off-top bits wrap to the bottom
    k %= 64
    return n & M if k == 0 else ((n << k) | (n >> (64 - k))) & M

def message(n, k):
    r = rotl(n, k)
    return f"rotl: {n} rotl {k} = {r} (0x{r:016x})"

cases = [(1, 1), (1, 63), (9223372036854775808, 1), (255, 8)]
for n, k in cases:
    want = message(n, k)   # value + hex DERIVED by python, never hand-typed
    if want not in s:
        print(f"ROTL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: LSB->up, high bit set, wrap-around, byte shift
assert message(1, 1) in s and "= 2 " in message(1, 1)
assert message(9223372036854775808, 1) in s and "= 1 " in message(9223372036854775808, 1)  # wrap
assert message(255, 8) in s and "= 65280 " in message(255, 8)

print("ROTL TEST PASSED  (1<<1=2, 1 rotl 63=2^63, 2^63 rotl 1 wraps to 1, 255 rotl 8=65280 "
      "vs python u64 rotate_left)")
PY
