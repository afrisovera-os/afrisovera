#!/usr/bin/env bash
# Verify `wagstaff <n>` computes the nth Wagstaff number W(n) = (2^n+1)/3 for ODD n in
# u128: W(1)=1, W(5)=11, W(7)=43, W(127)=56713727820156410577229101238628035243. Even n
# is undefined (2^n+1 not divisible by 3); W(129) overflows u128. A Mersenne cousin.
# Self-checking: python computes every reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/wagstaff.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wagstaff.png" \
    "echo warmup" "wagstaff 5" "wagstaff 4" "wagstaff 127" "wagstaff 129" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wagstaff" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):  # mirror the kernel: (2^n+1)/3 for odd n, else undefined; u128 guard
    if n % 2 == 0:
        return f"wagstaff: W({n}) is undefined (n must be odd)"
    if n >= 128:
        return f"wagstaff({n}) overflows u128"
    return f"wagstaff({n}) = {((1 << n) + 1) // 3}"

for n in [5, 4, 127, 129]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"WAGSTAFF TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "wagstaff(5) = 11" in s
assert "wagstaff: W(4) is undefined (n must be odd)" in s
assert "wagstaff(127) = 56713727820156410577229101238628035243" in s
assert "wagstaff(129) overflows u128" in s

print("WAGSTAFF TEST PASSED  (W(5)=11, W(4) undefined (even), W(127)=big, W(129) overflows "
      "u128 vs python (2^n+1)/3)")
PY
