#!/usr/bin/env bash
# Verify the `censor <text>` command masks each whitespace-separated word for privacy:
# keep its first character, replace every other character with '*'. "hello world" ->
# "h**** w****", "a bb ccc" -> "a b* c**". Distinct from disemvowel/strippunct (which
# filter chars out) and numeronym (which abbreviates with a middle count). Self-checking:
# python replicates the per-word mask.
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
OUT="${TMPDIR:-/tmp}/censor.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "a bb ccc" pins the varying word lengths; "x" the single-char (unmasked) case.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/censor.png" \
    "echo warmup" "censor hello world" "censor a bb ccc" "censor x" "censor AfriSovera OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "censor:|usage: censor" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def censor(t):
    return " ".join(w[0] + "*" * (len(w) - 1) for w in t.split())

for t in ["hello world", "a bb ccc", "x", "AfriSovera OS"]:
    want = f"censor: {censor(t)}"
    if want not in s:
        print(f"CENSOR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "censor: h**** w****" in s
assert "censor: a b* c**" in s              # varying word lengths
assert "censor: x" in s                     # single char -> unmasked
assert "censor: A********* O*" in s          # AfriSovera=10 -> A+9 stars

print("CENSOR TEST PASSED  (hello world->h**** w****, a bb ccc->a b* c**, x->x, "
      "AfriSovera OS->A********* O* vs python; per-word first-letter mask)")
PY
