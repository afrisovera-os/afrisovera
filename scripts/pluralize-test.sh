#!/usr/bin/env bash
# Verify the `pluralize <text>` command applies naive English pluralization per word:
# s/x/z or ch/sh -> +es (box->boxes, dish->dishes, bus->buses); consonant+y -> ies
# (city->cities); vowel+y or anything else -> +s (day->days, cat->cats). The stem's
# case is preserved (AfriSovera->AfriSoveras). Self-checking: python mirrors the rule.
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
OUT="${TMPDIR:-/tmp}/pluralize.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pluralize.png" \
    "echo warmup" "pluralize cat box city" "pluralize day dish bus" "pluralize AfriSovera" "pluralize" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pluralize:|usage: pluralize" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def pl(w):  # mirror the kernel rule exactly
    lc = lambda x: x.lower()
    n = len(w); last = lc(w[-1]); prev = lc(w[-2]) if n >= 2 else ''
    vowel = prev in 'aeiou'
    es = last in 'sxz' or (n >= 2 and prev == 'c' and last == 'h') or (n >= 2 and prev == 's' and last == 'h')
    ies = last == 'y' and n >= 2 and not vowel
    stem = w[:-1] if ies else w
    return stem + ('ies' if ies else ('es' if es else 's'))

def plural(phrase):
    return " ".join(pl(w) for w in phrase.split())

for phrase in ["cat box city", "day dish bus", "AfriSovera"]:
    want = f"pluralize: {plural(phrase)}"
    if want not in s:
        print(f"PLURALIZE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable branches explicitly (values derived above)
assert "pluralize: cats boxes cities" in s     # s / x->es / consonant+y->ies
assert "pluralize: days dishes buses" in s      # vowel+y->s / sh->es / s->es
assert "pluralize: AfriSoveras" in s            # stem case preserved
assert "usage: pluralize <text>" in s           # empty-arg usage branch

print("PLURALIZE TEST PASSED  (cat box city->cats boxes cities, day dish bus->days dishes buses, "
      "AfriSovera->AfriSoveras vs python; usage branch OK)")
PY
