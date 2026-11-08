#!/usr/bin/env bash
# Verify `scrabblef <file>` sums the English Scrabble tile values of all letters in a VFS file
# (file analogue of scrabble): motd and about.txt score their seeded content; a missing file
# errors. Self-checking: python scores the known seed content with the Scrabble table.
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
OUT="${TMPDIR:-/tmp}/scrabblef.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/scrabblef.png" \
    "echo warmup" "scrabblef motd" "scrabblef about.txt" \
    "scrabblef nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "scrabblef:|usage: scrabblef" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# exact seed content from vfs::seed()
MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

VAL = {}
for letters, v in [("aeioulnstr",1),("dg",2),("bcmp",3),("fhvwy",4),("k",5),("jx",8),("qz",10)]:
    for c in letters: VAL[c] = v

def score(text):
    return sum(VAL[c] for c in text.lower() if c in VAL)     # DERIVED from seed content

want_motd  = f"scrabblef: motd scores {score(MOTD)}"
want_about = f"scrabblef: about.txt scores {score(ABOUT)}"
for want in (want_motd, want_about):
    if want not in s:
        print(f"SCRABBLEF TEST FAILED (missing '{want}')"); sys.exit(1)

assert "scrabblef: no such file: nope.txt" in s

print(f"SCRABBLEF TEST PASSED  (motd->{score(MOTD)}, about.txt->{score(ABOUT)}, nope.txt errors "
      "vs python Scrabble score of the seed content)")
PY
