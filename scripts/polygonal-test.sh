#!/usr/bin/env bash
# Verify `polygonal <s> <n>` computes the nth s-gonal number P(s,n)=n+(s-2)n(n-1)/2: P(3,5)=15
# (triangular), P(4,5)=25 (square), P(5,5)=35 (pentagonal), P(6,4)=28 (hexagonal), P(10,3)=27
# (decagonal). Generalizes the figurate family. Self-checking: python n+(s-2)*n*(n-1)//2 with
# the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/polygonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/polygonal.png" \
    "echo warmup" "polygonal 3 5" "polygonal 4 5" "polygonal 5 5" \
    "polygonal 6 4" "polygonal 10 3" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "polygonal:|usage: polygonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

U128 = (1 << 128) - 1
def line(sides, n):
    v = n + (sides - 2) * n * (n - 1) // 2       # DERIVED general polygonal formula
    if v > U128:
        return f"polygonal: P({sides}, {n}) overflowed u128"
    return f"polygonal: P({sides}, {n}) = {v}"

for sides, n in [(3, 5), (4, 5), (5, 5), (6, 4), (10, 3)]:
    w = line(sides, n)
    if w not in s:
        print(f"POLYGONAL TEST FAILED (s={sides} n={n} missing '{w}')"); sys.exit(1)

# pin the notable cases matching the specific figurate families
assert "polygonal: P(3, 5) = 15" in s     # triangular(5)
assert "polygonal: P(4, 5) = 25" in s     # square(5)
assert "polygonal: P(5, 5) = 35" in s     # pentagonal(5)
assert "polygonal: P(6, 4) = 28" in s     # hexagonal(4)
assert "polygonal: P(10, 3) = 27" in s    # decagonal(3)

print("POLYGONAL TEST PASSED  (3/5->15, 4/5->25, 5/5->35, 6/4->28, 10/3->27 "
      "vs python n+(s-2)n(n-1)/2, matching the per-shape figurate families)")
PY
