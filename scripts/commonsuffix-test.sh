#!/usr/bin/env bash
# Verify `commonsuffix <a> <b>` finds the longest common suffix of two strings (mirror of
# commonprefix): "testing running" -> ing, "yellow fellow" -> ellow, "abc xyz" -> "" (empty),
# "same same" -> same, "cat hat" -> at. Distinct from stripsuffix/after. Self-checking: python
# compares byte by byte from the END.
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
OUT="${TMPDIR:-/tmp}/commonsuffix.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/commonsuffix.png" \
    "echo warmup" "commonsuffix testing running" "commonsuffix yellow fellow" "commonsuffix abc xyz" \
    "commonsuffix same same" "commonsuffix cat hat" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "commonsuffix:|usage: commonsuffix" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def cs(a, b):
    i = 0
    while i < len(a) and i < len(b) and a[-1 - i] == b[-1 - i]:   # DERIVED byte-wise from END
        i += 1
    return a[len(a) - i:]

cases = [("testing", "running"), ("yellow", "fellow"), ("abc", "xyz"),
         ("same", "same"), ("cat", "hat")]
for a, b in cases:
    want = "commonsuffix: " + cs(a, b)
    if want not in s:
        print(f"COMMONSUFFIX TEST FAILED (a={a} b={b} missing '{want}')"); sys.exit(1)

# pin the notable branches: -ing, -ellow, no overlap empty, identical, short -at
assert "commonsuffix: ing" in s
assert "commonsuffix: ellow" in s
assert "commonsuffix: same" in s
assert "commonsuffix: at" in s

print("COMMONSUFFIX TEST PASSED  (testing running->ing, yellow fellow->ellow, abc xyz->empty, "
      "same same->same, cat hat->at vs python byte-wise LCS-suffix)")
PY
