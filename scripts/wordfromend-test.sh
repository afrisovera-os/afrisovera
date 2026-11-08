#!/usr/bin/env bash
# Verify `wordfromend <n> <text>` returns the Nth word from the END: 1 a b c d->d, 2 a b c d->c,
# 4 a b c d->a, 5 a b c->(none), 1 solo->solo. End-anchored mirror of wordat. Self-checking:
# python words[-n].
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
OUT="${TMPDIR:-/tmp}/wordfromend.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordfromend.png" \
    "echo warmup" "wordfromend 1 a b c d" "wordfromend 2 a b c d" "wordfromend 4 a b c d" \
    "wordfromend 5 a b c" "wordfromend 1 solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordfromend:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    ws = text.split()
    return "wordfromend: " + (ws[-n] if 1 <= n <= len(ws) else "(none)")

cases = [(1, "a b c d"), (2, "a b c d"), (4, "a b c d"), (5, "a b c"), (1, "solo")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"WORDFROMEND TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: last, second-last, first-via-end, out-of-range, single
assert "wordfromend: d" in s
assert "wordfromend: c" in s
assert "wordfromend: a" in s          # 4th from end of 4 words = first
assert "wordfromend: (none)" in s     # out of range
assert "wordfromend: solo" in s

print("WORDFROMEND TEST PASSED  (1 a b c d->d, 2 a b c d->c, 4 a b c d->a, 5 a b c->(none), "
      "1 solo->solo vs python words[-n])")
PY
