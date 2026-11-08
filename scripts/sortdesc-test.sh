#!/usr/bin/env bash
# Verify `sortdesc <words...>` sorts whitespace args lexicographically DESCENDING -- the reverse of
# `sort`: "c a b"->c b a, "banana apple cherry"->cherry banana apple, "a"->a, "b b a"->b b a.
# Distinct from sort (ascending) / sortbylen (by length). Self-checking: python sorted(reverse=True).
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
OUT="${TMPDIR:-/tmp}/sortdesc.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sortdesc.png" \
    "echo warmup" "sortdesc c a b" "sortdesc banana apple cherry" "sortdesc a" \
    "sortdesc b b a" "sortdesc dog cat bird ant" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sorted:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference: descending byte-order sort
    ws = sorted(text.split(), reverse=True)
    return "sorted: " + " ".join(ws)

cases = ["c a b", "banana apple cherry", "a", "b b a", "dog cat bird ant"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"SORTDESC TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: reorder, real words, single, dup-with-tie, four words
assert "sorted: c b a" in s
assert "sorted: cherry banana apple" in s
assert "sorted: a" in s
assert "sorted: b b a" in s
assert "sorted: dog cat bird ant" in s

print("SORTDESC TEST PASSED  (c a b->c b a, banana apple cherry->cherry banana apple, a->a, "
      "b b a->b b a, dog cat bird ant->dog cat bird ant vs python sorted(reverse=True))")
PY
