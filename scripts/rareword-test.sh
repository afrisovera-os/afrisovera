#!/usr/bin/env bash
# Verify `rareword <text>` reports the LEAST frequent word + count, ties->first: "a b a c a"->b (1),
# "x y z"->x (1), "one two two three three"->one (1), "hi hi bye"->bye (1), "aa bb aa"->bb (1).
# Complement of topword. Self-checking: python first-occ min count.
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
OUT="${TMPDIR:-/tmp}/rareword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rareword.png" \
    "echo warmup" "rareword a b a c a" "rareword x y z" "rareword one two two three three" \
    "rareword hi hi bye" "rareword aa bb aa" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rareword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference
    ws = text.split()
    best, best_c = "", None
    for i, w in enumerate(ws):
        if w in ws[:i]:
            continue
        c = ws.count(w)
        if best_c is None or c < best_c:
            best_c, best = c, w
    return f"rareword: {best} ({best_c})"

cases = ["a b a c a", "x y z", "one two two three three", "hi hi bye", "aa bb aa"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"RAREWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: singleton among repeats, all-tie->first, min-with-repeats, one-off, single
assert "rareword: b (1)" in s
assert "rareword: x (1)" in s          # all tie -> first
assert "rareword: one (1)" in s
assert "rareword: bye (1)" in s
assert "rareword: bb (1)" in s

print("RAREWORD TEST PASSED  (a b a c a->b (1), x y z->x (1), one two two three three->one (1), "
      "hi hi bye->bye (1), aa bb aa->bb (1) vs python first-occ min count)")
PY
