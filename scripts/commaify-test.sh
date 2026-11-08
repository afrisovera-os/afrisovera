#!/usr/bin/env bash
# Verify `commaify <n>` inserts thousands separators into a non-negative integer, grouping
# decimal digits in threes from the right: 1234567 -> 1,234,567; 1000 -> 1,000; 100 -> 100;
# 0 -> 0; and the u64 maximum. A pure formatting util (distinct from cardinal, which spells
# the number). Self-checking: python groups each reference number with its own formatter.
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
OUT="${TMPDIR:-/tmp}/commaify.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/commaify.png" \
    "echo warmup" "commaify 1234567" "commaify 1000" "commaify 100" \
    "commaify 0" "commaify 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "commaify:|usage: commaify" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# python's own thousands grouping ("{:,}") is the reference -- every expected value DERIVED,
# never hand-typed. Includes the u64 maximum 2**64 - 1.
cases = [1234567, 1000, 100, 0, 18446744073709551615]
for n in cases:
    want = "commaify: " + format(n, ",")
    if want not in s:
        print(f"COMMAIFY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: multi-group, group boundary, no-group, zero, u64 max
assert "commaify: 1,234,567" in s
assert "commaify: 1,000" in s
assert "commaify: 100" in s
assert "commaify: 0" in s
assert "commaify: 18,446,744,073,709,551,615" in s   # u64::MAX

print("COMMAIFY TEST PASSED  (1234567->1,234,567, 1000->1,000, 100 unchanged, 0->0, "
      "u64 max grouped vs python '{:,}' formatter)")
PY
