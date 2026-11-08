#!/usr/bin/env bash
# Verify the `firstword <file>` command prints the FIRST whitespace-separated
# word of EACH line of a VFS file, tagged by 1-based line number. Distinct from
# wordn (Nth word of the whole file), line (a line's content), head (whole
# lines), and maxword/minword. Self-checking: python re-derives the per-line
# first word from the known seed contents.
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
OUT="${TMPDIR:-/tmp}/firstword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstword.png" \
    "echo warmup" "firstword motd" "firstword about.txt" \
    "firstword nope" "firstword" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstword:|usage: firstword" "$OUT" | sed 's/^/  /'

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
        want = f"firstword: line {i}: {w[0] if w else '(none)'}"
        if want not in s:
            print(f"FIRSTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly
for want in [
    "firstword: line 1: Welcome",       # motd
    "firstword: line 1: AfriSovera:",    # about.txt line 1 (keeps trailing ':')
    "firstword: line 2: AI",             # about.txt line 2
    "firstword: no such file: nope",     # missing file
    "usage: firstword <file>",           # no arg
]:
    if want not in s:
        print(f"FIRSTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

print("FIRSTWORD TEST PASSED  (motd->Welcome; about.txt->AfriSovera:/AI vs python; "
      "missing-file + no-arg handled)")
PY
