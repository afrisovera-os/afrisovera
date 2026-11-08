#!/usr/bin/env bash
# Verify the `letterfreq <file>` command reports the most common ASCII letter
# (case-insensitive; A-Z folded to a-z) in a VFS file, with its count and the
# total letter count. Ties are broken alphabetically-first (the kernel scans a-z
# and keeps the first index reaching the max). DISTINCT from histogram (most
# common BYTE), wordfreq (WORD), charcount (a specific char), vowelcount (vowels
# vs letters), mode (NUMBER). Self-checking: python re-derives from the seeds.
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
OUT="${TMPDIR:-/tmp}/letterfreq.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/letterfreq.png" \
    "echo warmup" "letterfreq about.txt" "letterfreq motd" \
    "letterfreq nofile.txt" "letterfreq" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "letterfreq:|usage: letterfreq" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def line(name, text):
    freq = {}
    for c in text.lower():
        if c.isalpha() and c.isascii():
            freq[c] = freq.get(c, 0) + 1
    total = sum(freq.values())
    mx = max(freq.values())
    winner = min(l for l, c in freq.items() if c == mx)   # alphabetically-first tie-break
    return f"letterfreq: {name} = most common letter '{winner}' ({freq[winner]} of {total})"

for name, text in [("about.txt", ABOUT), ("motd", MOTD)]:
    want = line(name, text)
    if want not in s:
        print(f"LETTERFREQ TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the headline results
assert line("about.txt", ABOUT) == "letterfreq: about.txt = most common letter 'e' (17 of 100)"
assert line("motd", MOTD) == "letterfreq: motd = most common letter 'e' (7 of 50)"

if "letterfreq: no such file: nofile.txt" not in s:
    print("LETTERFREQ TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: letterfreq <file>" not in s:
    print("LETTERFREQ TEST FAILED (missing usage line)"); sys.exit(1)

print("LETTERFREQ TEST PASSED  (about.txt -> 'e' (17 of 100), motd -> 'e' (7 of 50) "
      "re-derived vs python; not-found + usage)")
PY
