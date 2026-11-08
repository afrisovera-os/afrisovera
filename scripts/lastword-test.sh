#!/usr/bin/env bash
# Verify the `lastword <file>` command prints the LAST whitespace-separated
# word of EACH line of a VFS file, tagged by 1-based line number -- the mirror
# of firstword. Distinct from wordn (Nth word of the whole file), line (a
# line's content), tail (whole lines), and maxword/minword. Self-checking:
# python re-derives the per-line last word from the known seed contents.
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
OUT="${TMPDIR:-/tmp}/lastword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastword.png" \
    "echo warmup" "lastword motd" "lastword about.txt" \
    "lastword nope" "lastword" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastword:|usage: lastword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed contents from kernel/src/vfs.rs seed().
seed = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

for name, text in seed.items():
    for i, l in enumerate(text.split("\n"), 1):
        w = l.split()
        want = f"lastword: line {i}: {w[-1] if w else '(none)'}"
        if want not in s:
            print(f"LASTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly (punctuation is kept per whitespace-split)
for want in [
    "lastword: line 1: OS.",         # motd
    "lastword: line 1: store,",       # about.txt line 1 (keeps trailing ',')
    "lastword: line 2: it.",          # about.txt line 2
    "lastword: no such file: nope",   # missing file
    "usage: lastword <file>",         # no arg
]:
    if want not in s:
        print(f"LASTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

print("LASTWORD TEST PASSED  (motd->OS.; about.txt->store,/it. vs python; "
      "missing-file + no-arg handled)")
PY
