#!/usr/bin/env bash
# Verify `gcd3 <a> <b> <c>` gives gcd of three integers: 12 18 24->6, 100 40 60->20, 8 12 20->4,
# 7 5 3->1 (coprime), 30 42 70->2. Three-arg extension of gcd. Self-checking: python gcd(gcd(a,b),c).
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
OUT="${TMPDIR:-/tmp}/gcd3.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/gcd3.png" \
    "echo warmup" "gcd3 12 18 24" "gcd3 100 40 60" "gcd3 8 12 20" \
    "gcd3 7 5 3" "gcd3 30 42 70" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "gcd3" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b, c):                    # DERIVED reference
    return f"gcd3({a}, {b}, {c}) = {math.gcd(math.gcd(a, b), c)}"

for a, b, c in [(12, 18, 24), (100, 40, 60), (8, 12, 20), (7, 5, 3), (30, 42, 70)]:
    w = want(a, b, c)
    if w not in s:
        print(f"GCD3 TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: common 6, common 20, common 4, coprime 1, common 2
assert "gcd3(12, 18, 24) = 6" in s
assert "gcd3(100, 40, 60) = 20" in s
assert "gcd3(8, 12, 20) = 4" in s
assert "gcd3(7, 5, 3) = 1" in s          # pairwise-ish coprime -> 1
assert "gcd3(30, 42, 70) = 2" in s

print("GCD3 TEST PASSED  (12 18 24->6, 100 40 60->20, 8 12 20->4, 7 5 3->1, 30 42 70->2 "
      "vs python gcd(gcd(a,b),c))")
PY
