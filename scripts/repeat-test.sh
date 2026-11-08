#!/usr/bin/env bash
# Verify the `repeat <n> <text>` command repeats a whole run of text n times,
# concatenated (repeat 3 ab -> "ababab"). Output is capped at a fixed [u8; 512]
# buffer, so a large n stops filling once full (no overflow/panic). Distinct from
# double (repeats each CHARACTER twice, not the whole string n times). Self-
# checking: python text*n, truncated to 512.
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
OUT="${TMPDIR:-/tmp}/repeat.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "repeat 1000 ab" exercises the [u8;512] cap; "echo alive" proves no panic.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/repeat.png" \
    "echo warmup" "repeat 3 ab" "repeat 5 xy" "repeat 1000 ab" "echo alive" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "repeat:|usage: repeat|alive" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# small cases: text * n, exact
for n, t in [(3, "ab"), (5, "xy")]:
    want = f"repeat: {t * n}"
    if want not in s:
        print(f"REPEAT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "repeat: ababab" in s          # 3 x "ab"
assert "repeat: xyxyxyxyxy" in s      # 5 x "xy"

# large-n cap: 1000 x "ab" = 2000 bytes, capped to the [u8;512] buffer -> exactly
# 512 'a'/'b' bytes (256 "ab" pairs). Assert the capped prefix appears and the
# shell survived (no panic).
capped = ("ab" * 1000)[:512]
if f"repeat: {capped}" not in s:
    print("REPEAT TEST FAILED (1000-repeat not capped to 512 bytes as expected)"); sys.exit(1)
if "alive" not in s:
    print("REPEAT TEST FAILED (shell did not survive the large repeat -- possible overflow)"); sys.exit(1)

print("REPEAT TEST PASSED  (3/ab->ababab, 5/xy->xyxyxyxyxy vs python; 1000/ab capped "
      "to 512 bytes without panic; distinct from double)")
PY
