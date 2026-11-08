#!/usr/bin/env bash
# Verify the `thabit <n>` command computes the nth Thabit number T(n) = 3*2^n - 1
# (the "321 numbers": T(0)=2, T(1)=5, T(2)=11, T(3)=23, ...). T(126) is the largest
# that fits u128; T(127) overflows. Self-checking: python computes 3*2**n - 1 (the
# big value for n=126 is derived by the loop, NOT hand-typed).
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
OUT="${TMPDIR:-/tmp}/thabit.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# n=0 (=2), n=3 (=23), n=126 (largest that fits u128), n=127 (overflow).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/thabit.png" \
    "echo warmup" "thabit 0" "thabit 3" "thabit 126" "thabit 127" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "thabit\(|usage: thabit" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def thabit(n):
    return 3 * (2 ** n) - 1

for n in [0, 3, 126]:
    want = f"thabit({n}) = {thabit(n)}"      # big literal DERIVED by python, never hand-typed
    if want not in s:
        print(f"THABIT TEST FAILED (missing '{want}')"); sys.exit(1)

# small pins + the overflow path
assert "thabit(0) = 2" in s
assert "thabit(3) = 23" in s
assert "thabit(127) overflows u128" in s     # 3*2^127 > u128::MAX

print("THABIT TEST PASSED  (T(0)=2, T(3)=23, T(126) fits vs python, T(127) overflows; "
      "3*2^n - 1, u128 guarded shift)")
PY
