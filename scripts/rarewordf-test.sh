#!/usr/bin/env bash
# Verify `rarewordf <file>` reports the LEAST frequent whitespace-delimited word of a VFS file with its
# count and total (ties -> first occurrence) -- the complement of wordfreq and file analogue of
# rareword. about.txt -> "AfriSovera:" (1 of 22) (while wordfreq gives "the" 2 of 22), motd ->
# "Welcome" (1 of 9). Missing file errors. Self-checking: python derives min-count first word.
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
OUT="${TMPDIR:-/tmp}/rarewordf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rarewordf.png" \
    "echo warmup" "rarewordf motd" "rarewordf about.txt" "rarewordf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rarewordf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rarewordf(content):               # DERIVED reference: first word with the lowest count
    words = content.split()
    total = len(words)
    best, best_count = "", None
    for i, w in enumerate(words):
        if w in words[:i]:
            continue
        c = words.count(w)
        if best_count is None or c < best_count:
            best_count, best = c, w
    return f"rarewordf: least common word '{best}' ({best_count} of {total})"

want_motd, want_about = rarewordf(MOTD), rarewordf(ABOUT)
for w in (want_motd, want_about):
    if w not in s:
        print(f"RAREWORDF TEST FAILED (missing '{w}')"); sys.exit(1)

assert "rarewordf: least common word 'Welcome' (1 of 9)" in s
assert "rarewordf: least common word 'AfriSovera:' (1 of 22)" in s
assert "rarewordf: no such file: nope.txt" in s

print("RAREWORDF TEST PASSED  (motd -> Welcome (1 of 9), about.txt -> AfriSovera: (1 of 22) "
      "[wordfreq would give 'the' 2 of 22], nope.txt errors vs python min-count first word)")
PY
