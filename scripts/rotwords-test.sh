#!/usr/bin/env bash
# Verify the `rotwords <n> <text>` command circular LEFT-rotates the word sequence by n:
# output = words[k..] ++ words[0..k], k = n mod word_count. n >= count wraps. The
# word-level analogue of rotate (which rotates characters); distinct from rwords (which
# reverses word order). Self-checking: python replicates the rotation including the wrap.
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
OUT="${TMPDIR:-/tmp}/rotwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "5 a b c" pins the modulo wrap (5 mod 3 = 2); "0 x y z" the identity rotation.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotwords.png" \
    "echo warmup" "rotwords 1 a b c d" "rotwords 2 a b c d" "rotwords 5 a b c" "rotwords 0 x y z" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotwords:|usage: rotwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rot(n, t):
    w = t.split()
    if not w:
        return None
    k = n % len(w)
    return " ".join(w[k:] + w[:k])

cases = [(1, "a b c d"), (2, "a b c d"), (5, "a b c"), (0, "x y z")]
for n, t in cases:
    want = f"rotwords: {rot(n, t)}"
    if want not in s:
        print(f"ROTWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "rotwords: b c d a" in s        # 1 a b c d
assert "rotwords: c d a b" in s        # 2 a b c d
assert "rotwords: c a b" in s          # 5 a b c -> 5 mod 3 = 2 (wrap)
assert "rotwords: x y z" in s          # 0 -> identity

print("ROTWORDS TEST PASSED  (1 a b c d->b c d a, 2->c d a b, 5 a b c->c a b (mod wrap), "
      "0 x y z->x y z vs python; word-level rotation, distinct from rotate (chars) / rwords (reverse))")
PY
