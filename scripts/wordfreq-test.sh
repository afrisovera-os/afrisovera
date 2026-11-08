#!/usr/bin/env bash
# Verify the `wordfreq <file>` command reports the MOST FREQUENT whitespace-
# delimited word in a VFS file (case-sensitive; punctuation kept in the token,
# first-encountered wins ties). DISTINCT from histogram (BYTE frequency), mode
# (most common NUMBER), wordn (the Nth word), and wc (counts). The seeded
# about.txt contains "the" exactly twice ("the device and the software") out of
# 22 tokens, so it is the unique most-common word. Self-checking: python
# re-derives the winner + counts from the seed strings.
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
OUT="${TMPDIR:-/tmp}/wordfreq.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordfreq.png" \
    "echo warmup" "wordfreq about.txt" "wordfreq motd" \
    "wordfreq nofile.txt" "wordfreq" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordfreq:|usage: wordfreq" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def winner(text):
    toks = text.split()
    best, best_c = "", 0
    for t in toks:                    # first-encountered wins ties (strict >)
        c = toks.count(t)
        if c > best_c:
            best_c, best = c, t
    return best, best_c, len(toks)

for label, text in [("about.txt", ABOUT), ("motd", MOTD)]:
    w, c, tot = winner(text)
    want = f"wordfreq: most common word '{w}' ({c} of {tot})"
    if want not in s:
        print(f"WORDFREQ TEST FAILED (missing '{want}' for {label})"); sys.exit(1)

# pin the headline case: about.txt -> 'the' (2 of 22)
assert winner(ABOUT) == ("the", 2, 22), winner(ABOUT)

if "wordfreq: no such file: nofile.txt" not in s:
    print("WORDFREQ TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: wordfreq <file>" not in s:
    print("WORDFREQ TEST FAILED (missing usage line)"); sys.exit(1)

print("WORDFREQ TEST PASSED  (about.txt -> 'the' (2 of 22), motd first-token tie, "
      "not-found + usage; re-derived vs python)")
PY
