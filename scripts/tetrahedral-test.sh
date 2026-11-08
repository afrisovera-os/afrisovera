#!/usr/bin/env bash
# Verify the `tetrahedral <n>` command computes the nth tetrahedral number
# Te(n) = n*(n+1)*(n+2)/6 (the sum of the first n triangular numbers; spheres in a
# triangular pyramid). Te(1..7) = 1,4,10,20,35,56,84. The 3D analogue of sumto (nth
# triangular number); distinct from the figurate MEMBERSHIP tests. The triple product
# is computed in u128 (checked). Self-checking: python replicates the closed form.
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
OUT="${TMPDIR:-/tmp}/tetrahedral.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 0 pins the zero edge; 100 pins a larger value (u128 product path).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tetrahedral.png" \
    "echo warmup" "tetrahedral 4" "tetrahedral 7" "tetrahedral 100" "tetrahedral 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tetrahedral:|usage: tetrahedral" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def te(n):
    return n * (n + 1) * (n + 2) // 6

for n in [4, 7, 100, 0]:
    want = f"tetrahedral: Te({n}) = {te(n)}"
    if want not in s:
        print(f"TETRAHEDRAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "tetrahedral: Te(4) = 20" in s
assert "tetrahedral: Te(7) = 84" in s
assert "tetrahedral: Te(100) = 171700" in s    # 100*101*102/6
assert "tetrahedral: Te(0) = 0" in s

print("TETRAHEDRAL TEST PASSED  (Te(4)=20, Te(7)=84, Te(100)=171700, Te(0)=0 vs python; "
      "n(n+1)(n+2)/6, the 3D analogue of sumto)")
PY
