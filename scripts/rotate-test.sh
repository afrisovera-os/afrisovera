#!/usr/bin/env bash
# Verify the `rotate <n> <text>` command circular LEFT-rotates the characters of the
# text by n positions: output = text[k..] ++ text[0..k], k = n mod len. n >= len wraps.
# Distinct from caesar (shifts letter VALUES), rev (reverses), swappairs/swapends
# (rearrange pairs/ends) -- this cyclically shifts character POSITIONS. Self-checking:
# python replicates the rotation including the modulo wrap.
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
OUT="${TMPDIR:-/tmp}/rotate.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "7 abc" pins the modulo wrap (7 mod 3 = 1); "0 world" pins the identity rotation.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotate.png" \
    "echo warmup" "rotate 2 hello" "rotate 7 abc" "rotate 0 world" "rotate 3 abcdef" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotate:|usage: rotate" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rot(n, t):
    if len(t) == 0:
        return None
    k = n % len(t)
    return t[k:] + t[:k]

cases = [(2, "hello"), (7, "abc"), (0, "world"), (3, "abcdef")]
for n, t in cases:
    want = f"rotate: {rot(n, t)}"
    if want not in s:
        print(f"ROTATE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "rotate: llohe" in s            # 2 hello
assert "rotate: bca" in s              # 7 abc -> 7 mod 3 = 1 (wrap)
assert "rotate: world" in s            # 0 world -> identity
assert "rotate: defabc" in s           # 3 abcdef

print("ROTATE TEST PASSED  (2 hello->llohe, 7 abc->bca (mod wrap), 0 world->world, "
      "3 abcdef->defabc vs python; character-position rotation, distinct from caesar/rev)")
PY
