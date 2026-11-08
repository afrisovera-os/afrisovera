#!/usr/bin/env bash
# Verify the `avgwordlen <file>` command reports the average word length (sum of word
# byte lengths / word count, integer division) of a VFS file. The word-level companion
# of avglen (average line length). Self-checking: python computes the totals from the
# exact seed content (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/avgwordlen.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/avgwordlen.png" \
    "echo warmup" "avgwordlen motd" "avgwordlen about.txt" "avgwordlen nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "avgwordlen:|usage: avgwordlen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed file contents (from kernel/src/vfs.rs seed()).
FILES = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def message(fname):
    words = FILES[fname].split()
    n = len(words)
    b = sum(len(w) for w in words)           # totals DERIVED by python, never hand-typed
    return f"avgwordlen: {n} words, {b} bytes, average {b // n} bytes/word"

for fname in ["motd", "about.txt"]:
    want = message(fname)
    if want not in s:
        print(f"AVGWORDLEN TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: motd has 9 words; about.txt has 22 words (8 + 14)
assert len(FILES["motd"].split()) == 9
assert len(FILES["about.txt"].split()) == 22
assert "avgwordlen: no such file: nope.txt" in s

print("AVGWORDLEN TEST PASSED  (motd 9 words + about.txt 22 words with computed avg vs python; "
      "missing-file handled; word-level companion of avglen)")
PY
