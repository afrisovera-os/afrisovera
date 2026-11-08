#!/usr/bin/env bash
# Verify `firstunique <text>` reports the first non-repeating character (case-sensitive):
# swiss->w, leetcode->l, AfriSovera->A; aabbcc has none. Distinct from uniqchars (lists
# distinct bytes). Self-checking: python replicates the first-count==1 scan.
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
OUT="${TMPDIR:-/tmp}/firstunique.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstunique.png" \
    "echo warmup" "firstunique swiss" "firstunique leetcode" "firstunique aabbcc" "firstunique AfriSovera" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstunique:|usage: firstunique" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from collections import Counter
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(text):  # mirror the kernel: first byte with count == 1, else none
    c = Counter(text)
    for ch in text:
        if c[ch] == 1:
            return f"firstunique: {ch}"
    return "firstunique: (no unique character)"

for t in ["swiss", "leetcode", "aabbcc", "AfriSovera"]:
    want = message(t)
    if want not in s:
        print(f"FIRSTUNIQUE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "firstunique: w" in s                       # swiss (s repeats)
assert "firstunique: l" in s                        # leetcode
assert "firstunique: (no unique character)" in s    # aabbcc
assert "firstunique: A" in s                        # AfriSovera (case-sensitive)

print("FIRSTUNIQUE TEST PASSED  (swiss->w, leetcode->l, aabbcc->none, AfriSovera->A "
      "vs python first count==1)")
PY
