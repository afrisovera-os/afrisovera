#!/usr/bin/env bash
# Verify the `maxword <file>` command reports the longest whitespace-delimited
# word (by byte length; FIRST occurrence wins ties) in a VFS file, with its
# length. DISTINCT from longestword (inline text), wordfreq (most common word),
# wordlengths (all inline lengths), wc (counts). Self-checking: python re-derives
# the first-wins longest token from the seeds.
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
OUT="${TMPDIR:-/tmp}/maxword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/maxword.png" \
    "echo warmup" "maxword about.txt" "maxword motd" \
    "maxword nofile.txt" "maxword" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "maxword:|usage: maxword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def line(name, text):
    toks = text.split()
    mx = max(len(t) for t in toks)
    best = next(t for t in toks if len(t) == mx)   # first-wins tie-break
    return f"maxword: {name} = longest word '{best}' ({mx} chars)"

for name, text in [("about.txt", ABOUT), ("motd", MOTD)]:
    want = line(name, text)
    if want not in s:
        print(f"MAXWORD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the headline results (about.txt has an 11-char tie -> first wins)
assert line("about.txt", ABOUT) == "maxword: about.txt = longest word 'AfriSovera:' (11 chars)"
assert line("motd", MOTD) == "maxword: motd = longest word 'capability-secured' (18 chars)"

if "maxword: no such file: nofile.txt" not in s:
    print("MAXWORD TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: maxword <file>" not in s:
    print("MAXWORD TEST FAILED (missing usage line)"); sys.exit(1)

print("MAXWORD TEST PASSED  (about.txt -> 'AfriSovera:' (11, first-wins tie), motd "
      "-> 'capability-secured' (18); not-found + usage)")
PY
