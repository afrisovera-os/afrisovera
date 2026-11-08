#!/usr/bin/env bash
# Verify the `minword <file>` command reports the shortest whitespace-delimited
# word (by byte length; FIRST occurrence wins ties) in a VFS file, with its
# length -- the min-twin of `maxword` (2.295). DISTINCT from shortestword (inline
# text), wordfreq (most common word), wc (counts). Self-checking: python
# re-derives the first-wins shortest token from the seeds.
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
OUT="${TMPDIR:-/tmp}/minword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/minword.png" \
    "echo warmup" "minword about.txt" "minword motd" \
    "minword nofile.txt" "minword" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "minword:|usage: minword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def line(name, text):
    toks = text.split()
    mn = min(len(t) for t in toks)
    best = next(t for t in toks if len(t) == mn)   # first-wins tie-break
    return f"minword: {name} = shortest word '{best}' ({mn} chars)"

for name, text in [("about.txt", ABOUT), ("motd", MOTD)]:
    want = line(name, text)
    if want not in s:
        print(f"MINWORD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the headline results (about.txt's first 2-char token is 'no'; motd -> 'a')
assert line("about.txt", ABOUT) == "minword: about.txt = shortest word 'no' (2 chars)"
assert line("motd", MOTD) == "minword: motd = shortest word 'a' (1 chars)"

if "minword: no such file: nofile.txt" not in s:
    print("MINWORD TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: minword <file>" not in s:
    print("MINWORD TEST FAILED (missing usage line)"); sys.exit(1)

print("MINWORD TEST PASSED  (about.txt -> 'no' (2, first-wins), motd -> 'a' (1); "
      "not-found + usage)")
PY
