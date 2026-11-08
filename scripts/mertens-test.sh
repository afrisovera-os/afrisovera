#!/usr/bin/env bash
# Verify the `mertens <n>` command computes the Mertens function M(n) = sum of the
# Mobius function mu(k) for k=1..=n (M(1)=1, M(2)=0, M(3)=-1, M(5)=-2, M(10)=-1,
# M(100)=1). The running partial sum of mobius; distinct from mobius (a single value).
# Self-checking: python replicates M(n) by summing mu(k) (references computed, not
# hand-typed).
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
OUT="${TMPDIR:-/tmp}/mertens.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 100 exercises the longer O(n sqrt n) loop; 5 gives a negative running sum (-2).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mertens.png" \
    "echo warmup" "mertens 10" "mertens 5" "mertens 100" "mertens 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mertens:|usage: mertens" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def mu(n):
    x = n; dist = 0; d = 2
    while d * d <= x:
        if x % d == 0:
            dist += 1; e = 0
            while x % d == 0:
                x //= d; e += 1
            if e >= 2:
                return 0
        d += 1
    if x > 1:
        dist += 1
    return 1 if dist % 2 == 0 else -1

def M(n):
    return sum(mu(k) for k in range(1, n + 1))

for n in [10, 5, 100, 1]:
    want = f"mertens: M({n}) = {M(n)}"
    if want not in s:
        print(f"MERTENS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values (computed above, not hand-typed)
assert f"mertens: M(1) = {M(1)}" in s        # 1
assert f"mertens: M(5) = {M(5)}" in s        # -2
assert f"mertens: M(10) = {M(10)}" in s      # -1
assert f"mertens: M(100) = {M(100)}" in s    # 1

print("MERTENS TEST PASSED  (M(1)=1, M(5)=-2, M(10)=-1, M(100)=1 vs python; "
      "running sum of mobius mu(k), distinct from single-value mobius)")
PY
