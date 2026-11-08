#!/usr/bin/env bash
# Verify `titlecountf <file>` counts TITLE-CASE words (first char uppercase, no other uppercase) across
# a whole VFS file -- the file analogue of titlecount. Checks motd and about.txt plus a missing file.
# Self-checking: python replicates the title-case predicate over the seed files.
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
OUT="${TMPDIR:-/tmp}/titlecountf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/titlecountf.png" \
    "echo warmup" "titlecountf motd" "titlecountf about.txt" "titlecountf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "titlecountf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def title_word(w):
    return w[0].isupper() and w[0].isascii() and not any(c.isupper() for c in w[1:] if c.isascii())

def counts(t):
    ws = t.split()
    return sum(1 for w in ws if title_word(w)), len(ws)

for name, t in [("motd", MOTD), ("about.txt", ABOUT)]:
    tc, tot = counts(t)
    want = f"titlecountf: {tc} of {tot} words are title case"
    if want not in s:
        print(f"TITLECOUNTF TEST FAILED ({name}: missing '{want}')"); sys.exit(1)

# motd: only "Welcome" qualifies (AfriSovera/OS have interior/all caps); about has You, etc.
assert counts(MOTD)[0] == 1
assert "titlecountf: no such file: nope.txt" in s
print(f"TITLECOUNTF TEST PASSED  (motd {counts(MOTD)}, about.txt {counts(ABOUT)} vs python; nope.txt errors)")
PY
