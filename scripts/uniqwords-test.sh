#!/usr/bin/env bash
# Verify the `uniqwords <file>` command counts DISTINCT whitespace-delimited words
# in a VFS file (case-sensitive; punctuation kept in the token). DISTINCT from wc
# (total counts), wordfreq (the most common word), histogram (byte frequency), and
# dedup/unique (which remove duplicate LINES). The seeded about.txt has "the"
# twice out of 22 tokens -> 21 distinct; motd's 9 tokens are all distinct -> 9 of
# 9. Self-checking: python re-derives distinct/total from the seed strings.
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
OUT="${TMPDIR:-/tmp}/uniqwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/uniqwords.png" \
    "echo warmup" "uniqwords about.txt" "uniqwords motd" \
    "uniqwords nofile.txt" "uniqwords" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uniqwords:|usage: uniqwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def line(name, text):
    t = text.split()
    return f"uniqwords: {name} = {len(set(t))} distinct of {len(t)} words"

for name, text in [("about.txt", ABOUT), ("motd", MOTD)]:
    want = line(name, text)
    if want not in s:
        print(f"UNIQWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the headline numbers
assert line("about.txt", ABOUT) == "uniqwords: about.txt = 21 distinct of 22 words"
assert line("motd", MOTD) == "uniqwords: motd = 9 distinct of 9 words"

if "uniqwords: no such file: nofile.txt" not in s:
    print("UNIQWORDS TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: uniqwords <file>" not in s:
    print("UNIQWORDS TEST FAILED (missing usage line)"); sys.exit(1)

print("UNIQWORDS TEST PASSED  (about.txt 21/22 [the x2], motd 9/9, not-found + "
      "usage; re-derived vs python)")
PY
