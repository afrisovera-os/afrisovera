#!/usr/bin/env bash
# Verify the `cullen <n>` command computes the nth Cullen number C(n) = n*2^n + 1
# (C(0)=1, C(1)=3, C(2)=9, C(3)=25, C(4)=65, ...). Large n reports overflow instead
# of panicking. Self-checking: python computes n*2**n + 1 (the big value for n=120
# is derived by the loop, NOT hand-typed) and predicts the u128 overflow boundary.
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
OUT="${TMPDIR:-/tmp}/cullen.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# n=0 (=1), n=4 (=65), n=120 (fits u128), n=125 (overflows).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/cullen.png" \
    "echo warmup" "cullen 0" "cullen 4" "cullen 120" "cullen 125" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cullen\(|usage: cullen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

UMAX = 2 ** 128 - 1
def cullen(n):
    return n * (2 ** n) + 1

for n in [0, 4, 120]:
    c = cullen(n)
    assert c <= UMAX, f"expected {n} to fit u128"
    want = f"cullen({n}) = {c}"          # big literal DERIVED by python, never hand-typed
    if want not in s:
        print(f"CULLEN TEST FAILED (missing '{want}')"); sys.exit(1)

# n=125 overflows u128 -- python predicts the same boundary the kernel enforces
assert cullen(125) > UMAX
if "cullen(125) overflows u128" not in s:
    print("CULLEN TEST FAILED (missing 'cullen(125) overflows u128')"); sys.exit(1)

# small pins
assert "cullen(0) = 1" in s
assert "cullen(4) = 65" in s

print("CULLEN TEST PASSED  (C(0)=1, C(4)=65, C(120) fits vs python, C(125) overflows; "
      "n*2^n + 1, u128 guarded shift)")
PY
