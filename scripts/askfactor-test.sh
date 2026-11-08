#!/usr/bin/env bash
# Verify the Asili on-device assistant's NEW prime factorization (docs/09 AI pillar): "ask
# factorize 60" -> 60 = 2 x 2 x 3 x 5, "ask factors of 100" -> 100 = 2 x 2 x 5 x 5, "ask
# factorize 17" -> 17 = 17 (prime), "ask prime factors of 84" -> 84 = 2 x 2 x 3 x 7 (factorize
# wins over the primality check), "ask factorize 97" -> 97 = 97. On-device trial division.
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
OUT="${TMPDIR:-/tmp}/askfactor.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askfactor.png" \
    "echo warmup" "ask factorize 60" "ask factors of 100" "ask factorize 17" \
    "ask prime factors of 84" "ask factorize 97" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def factor(n):
    f, d, m = [], 2, n
    while d * d <= m:
        while m % d == 0:
            f.append(d); m //= d
        d += 1
    if m > 1:
        f.append(m)
    return f

def line(n):
    return f"[asili] {n} = " + " x ".join(str(x) for x in factor(n)) + " (ON-DEVICE)"   # DERIVED

for n in [60, 100, 17, 84, 97]:
    want = line(n)
    if want not in s:
        print(f"ASKFACTOR TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "[asili] 60 = 2 x 2 x 3 x 5 (ON-DEVICE)" in s
assert "[asili] 100 = 2 x 2 x 5 x 5 (ON-DEVICE)" in s
assert "[asili] 17 = 17 (ON-DEVICE)" in s          # prime -> itself
assert "[asili] 84 = 2 x 2 x 3 x 7 (ON-DEVICE)" in s   # "prime factors of" factorizes
assert "[asili] 97 = 97 (ON-DEVICE)" in s

print("ASKFACTOR TEST PASSED  (60->2x2x3x5, 100->2x2x5x5, 17->17, prime factors of 84->2x2x3x7, "
      "97->97 vs python factorization, on-device Asili)")
PY
