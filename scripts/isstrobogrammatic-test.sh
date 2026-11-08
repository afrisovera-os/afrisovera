#!/usr/bin/env bash
# Verify the `isstrobogrammatic <n>` command classifies strobogrammatic numbers:
# n reads the same after a 180-degree rotation, where 0/1/8 map to themselves and
# 6<->9 swap (69, 88, 818 IS; 6 alone rotates to 9 so NOT; 25 has an unrotatable
# digit). Distinct from palindrome/revnum. Self-checking: python replicates the glyph
# rotation + mirror check.
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
OUT="${TMPDIR:-/tmp}/isstrobogrammatic.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 69 (IS), 818 (IS), 6 (NOT, rotates to 9), 25 (NOT, 2 not rotatable).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isstrobogrammatic.png" \
    "echo warmup" "isstrobogrammatic 69" "isstrobogrammatic 818" "isstrobogrammatic 6" "isstrobogrammatic 25" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isstrobogrammatic:|usage: isstrobogrammatic" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ROT = {"0": "0", "1": "1", "8": "8", "6": "9", "9": "6"}
def isstrobo(n):
    t = str(n)
    for i, c in enumerate(t):
        r = ROT.get(c)
        if r is None or r != t[len(t) - 1 - i]:
            return False
    return True

def message(n):
    return f"isstrobogrammatic: {n} IS strobogrammatic" if isstrobo(n) \
        else f"isstrobogrammatic: {n} is NOT strobogrammatic"

for n in [69, 818, 6, 25]:
    want = message(n)
    if want not in s:
        print(f"ISSTROBOGRAMMATIC TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "isstrobogrammatic: 69 IS strobogrammatic" in s      # 6<->9 mirror
assert "isstrobogrammatic: 818 IS strobogrammatic" in s     # 8/1/8 self-mapping
assert "isstrobogrammatic: 6 is NOT strobogrammatic" in s   # 6 alone rotates to 9
assert "isstrobogrammatic: 25 is NOT strobogrammatic" in s  # 2 not rotatable

print("ISSTROBOGRAMMATIC TEST PASSED  (69 IS, 818 IS, 6 NOT (rotates to 9), 25 NOT (2 unrotatable) "
      "vs python; reads the same rotated 180 with 6<->9)")
PY
