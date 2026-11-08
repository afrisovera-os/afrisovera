#!/usr/bin/env bash
# Verify the `syllables <text>` command gives a naive syllable estimate = the number of
# maximal ASCII-vowel runs (consecutive vowels count once): "hello world"->3 (e,o,o),
# "beautiful"->3 (eau,i,u), "AfriSovera"->5, "queue"->1 (ueue is one run). Distinct from
# vowels (counts each vowel). Self-checking: python replicates the vowel-run rule.
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
OUT="${TMPDIR:-/tmp}/syllables.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/syllables.png" \
    "echo warmup" "syllables hello world" "syllables beautiful" "syllables AfriSovera" "syllables queue" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "syllables:|usage: syllables" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def syl(t):  # mirror the kernel: count maximal ASCII-vowel runs
    t = t.strip()
    count = 0; inv = False
    for ch in t:
        isv = ch.lower() in "aeiou" and ch.isalpha()
        if isv and not inv:
            count += 1
        inv = isv
    return count

for t in ["hello world", "beautiful", "AfriSovera", "queue"]:
    want = f"syllables: {t} = {syl(t)} syllable(s)"
    if want not in s:
        print(f"SYLLABLES TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "syllables: hello world = 3 syllable(s)" in s   # e, o, o
assert "syllables: beautiful = 3 syllable(s)" in s       # eau | i | u  (consecutive vowels merge)
assert "syllables: AfriSovera = 5 syllable(s)" in s      # A i o e a
assert "syllables: queue = 1 syllable(s)" in s           # ueue is a single run

print("SYLLABLES TEST PASSED  (hello world->3, beautiful->3 (eau merges), AfriSovera->5, "
      "queue->1 vs python vowel-run count)")
PY
