#!/usr/bin/env bash
# Verify `hapaxf <file>` lists a VFS file's HAPAX words -- those occurring EXACTLY ONCE across the
# whole file (newlines as whitespace) -- in first-occurrence order, single-spaced. The file analogue
# of hapax. motd's tokens are all unique (OS vs OS.) so hapaxf returns them all; about.txt repeats
# "the" so it is dropped. Plus a missing file. Self-checking: python Counter over split().
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
OUT="${TMPDIR:-/tmp}/hapaxf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hapaxf.png" \
    "echo warmup" "hapaxf motd" "hapaxf about.txt" "hapaxf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hapaxf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from collections import Counter
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def hapax(t):                        # DERIVED reference: words with count==1, first-occurrence order
    ws = t.split()
    c = Counter(ws)
    seen, out = set(), []
    for w in ws:
        if c[w] == 1 and w not in seen:
            seen.add(w); out.append(w)
    return " ".join(out)

want_motd = "hapaxf: " + hapax(MOTD)
if want_motd not in s:
    print(f"HAPAXF TEST FAILED (missing motd: {want_motd!r})"); sys.exit(1)

want_about = "hapaxf: " + hapax(ABOUT)
if want_about not in s:
    print(f"HAPAXF TEST FAILED (missing about: {want_about!r})"); sys.exit(1)

# about.txt repeats "the" -> it must NOT be in the hapax list
assert "the" not in hapax(ABOUT).split()
assert "the" in ABOUT.split()
assert "hapaxf: no such file: nope.txt" in s
print("HAPAXF TEST PASSED  (motd all-unique + about.txt drops repeated 'the' vs python; nope.txt errors)")
PY
