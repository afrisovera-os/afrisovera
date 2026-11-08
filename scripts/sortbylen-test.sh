#!/usr/bin/env bash
# Verify the `sortbylen <words...>` command STABLE-sorts whitespace-separated words by
# ascending LENGTH (shortest first); equal-length words keep their input order. Distinct
# from `sort` (alphabetical byte-order) and `sortchars` (sorts the characters within the
# text) -- a different sort KEY. Self-checking: python replicates the stable length sort
# and the contrast with alphabetical sort.
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
OUT="${TMPDIR:-/tmp}/sortbylen.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "the quick brown fox" pins stability (two length-3 and two length-5 words).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sortbylen.png" \
    "echo warmup" "sortbylen apple hi banana cat" "sortbylen the quick brown fox" "sort apple hi banana cat" "sortbylen solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sortbylen:|sorted:|usage: sortbylen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sortbylen(words):
    # python's sort is stable, keyed on length -> same as the kernel's stable insertion sort
    return sorted(words, key=len)

cases = ["apple hi banana cat", "the quick brown fox", "solo"]
for t in cases:
    want = "sortbylen: " + " ".join(sortbylen(t.split()))
    if want not in s:
        print(f"SORTBYLEN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results + the sort-KEY contrast with alphabetical `sort`
assert "sortbylen: hi cat apple banana" in s
assert "sortbylen: the fox quick brown" in s        # stable: the<fox (both 3), quick<brown (both 5)
assert "sortbylen: solo" in s
assert "sorted: apple banana cat hi" in s            # `sort` is alphabetical, NOT by length

print("SORTBYLEN TEST PASSED  (apple hi banana cat->hi cat apple banana (vs sort's "
      "apple banana cat hi), the quick brown fox->the fox quick brown (stable ties) vs python)")
PY
