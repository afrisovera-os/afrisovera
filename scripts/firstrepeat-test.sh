#!/usr/bin/env bash
# Verify `firstrepeat <text>` reports the first repeating character -- the first byte whose
# value was already seen earlier (case-sensitive): swiss->s (4th char), abca->a,
# AfriSovera->r; abcdef has none. The complement of firstunique. Self-checking: python
# replicates the single-pass seen-set scan.
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
OUT="${TMPDIR:-/tmp}/firstrepeat.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstrepeat.png" \
    "echo warmup" "firstrepeat swiss" "firstrepeat abca" "firstrepeat abcdef" "firstrepeat AfriSovera" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstrepeat:|usage: firstrepeat" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(text):  # mirror the kernel: first byte already seen, else none
    seen = set()
    for ch in text:
        if ch in seen:
            return f"firstrepeat: {ch}"
        seen.add(ch)
    return "firstrepeat: (no repeating character)"

for t in ["swiss", "abca", "abcdef", "AfriSovera"]:
    want = message(t)
    if want not in s:
        print(f"FIRSTREPEAT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "firstrepeat: s" in s                          # swiss (s repeats at index 3)
assert "firstrepeat: a" in s                           # abca
assert "firstrepeat: (no repeating character)" in s    # abcdef
assert "firstrepeat: r" in s                           # AfriSovera (r repeats)

print("FIRSTREPEAT TEST PASSED  (swiss->s, abca->a, abcdef->none, AfriSovera->r "
      "vs python first-already-seen)")
PY
