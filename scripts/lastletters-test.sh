#!/usr/bin/env bash
# Verify the `lastletters <text>` command concatenates the LAST character of each
# whitespace-separated word, original case preserved ("hello world" -> "od",
# "own your device" -> "nre", "AfriSovera OS" -> "aS"). The mirror of initials
# (first char); distinct from acronym (first char uppercased). Self-checking:
# python replicates "".join(w[-1] for w in t.split()).
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
OUT="${TMPDIR:-/tmp}/lastletters.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# Mixed case + punctuation + a single word exercise case-preservation + edge cases.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastletters.png" \
    "echo warmup" "lastletters hello world" "lastletters own your device" "lastletters AfriSovera OS" "lastletters solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastletters:|usage: lastletters" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lastletters(t):
    return "".join(w[-1] for w in t.split())

for t in ["hello world", "own your device", "AfriSovera OS", "solo"]:
    want = f"lastletters: {lastletters(t)}"
    if want not in s:
        print(f"LASTLETTERS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "lastletters: od" in s        # hello->o world->d
assert "lastletters: nre" in s       # own->n your->r device->e
assert "lastletters: aS" in s        # AfriSovera->a OS->S (case preserved)
assert "lastletters: o" in s         # single word solo -> o

print("LASTLETTERS TEST PASSED  (hello world->od, own your device->nre, AfriSovera OS->aS "
      "(case kept), solo->o vs python; last letter of each word)")
PY
