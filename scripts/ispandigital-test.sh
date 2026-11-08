#!/usr/bin/env bash
# Verify the `ispandigital <n>` command classifies (1-9) pandigital numbers: the
# decimal digits include every digit from 1 to 9 at least once (123456789 IS,
# 1234567890 IS -- the 0 is ignored, 12345 NOT missing 6). Distinct from isrepdigit
# (all digits same). Self-checking: python replicates the exact two-branch message.
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
OUT="${TMPDIR:-/tmp}/ispandigital.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 123456789 (IS), 1234567890 (IS, 0 ignored), 12345 (NOT missing 6), 5 (NOT missing 1).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ispandigital.png" \
    "echo warmup" "ispandigital 123456789" "ispandigital 1234567890" "ispandigital 12345" "ispandigital 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ispandigital:|usage: ispandigital" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):
    ds = set(str(n))
    missing = next((d for d in range(1, 10) if str(d) not in ds), 0)
    if missing == 0:
        return f"ispandigital: {n} IS pandigital (has every digit 1-9)"
    return f"ispandigital: {n} is NOT pandigital (missing {missing})"

for n in [123456789, 1234567890, 12345, 5]:
    want = message(n)
    if want not in s:
        print(f"ISPANDIGITAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "ispandigital: 123456789 IS pandigital (has every digit 1-9)" in s
assert "ispandigital: 1234567890 IS pandigital (has every digit 1-9)" in s  # extra 0 ignored
assert "ispandigital: 12345 is NOT pandigital (missing 6)" in s
assert "ispandigital: 5 is NOT pandigital (missing 1)" in s

print("ISPANDIGITAL TEST PASSED  (123456789 IS, 1234567890 IS (0 ignored), 12345 NOT (missing 6), "
      "5 NOT (missing 1) vs python; contains every digit 1-9)")
PY
