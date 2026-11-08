#!/usr/bin/env bash
# Verify `tailwords <n> <file>` prints the LAST n whitespace-delimited words of a VFS file
# (all of them if fewer than n): about.txt 3 -> "software on it.", about.txt 1 -> "it.",
# motd 2 -> "capability-secured OS.", motd 100 -> whole file (9 words), missing file errors.
# Word-tail mirror of taillines. Self-checking: python's content.split()[-n:] is the ref.
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
OUT="${TMPDIR:-/tmp}/tailwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tailwords.png" \
    "echo warmup" "tailwords 3 about.txt" "tailwords 1 about.txt" \
    "tailwords 2 motd" "tailwords 100 motd" "tailwords 5 nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tailwords:|usage: tailwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# Exact seed content from vfs::seed() (kernel/src/vfs.rs).
MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def tail(content, n):
    return "tailwords: " + " ".join(content.split()[-n:] if n else [])   # DERIVED via python

checks = [tail(ABOUT, 3), tail(ABOUT, 1), tail(MOTD, 2), tail(MOTD, 100)]
for want in checks:
    if want not in s:
        print(f"TAILWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: last-3 across the line break, last-1, last-2, n>count (whole file)
assert "tailwords: software on it." in s
assert "tailwords: it." in s
assert "tailwords: capability-secured OS." in s
assert "tailwords: Welcome to AfriSovera OS -- a sovereign, capability-secured OS." in s
assert "tailwords: no such file: nope.txt" in s

print("TAILWORDS TEST PASSED  (about.txt 3->software on it., 1->it., motd 2->capability-secured OS., "
      "motd 100->whole file, nope.txt errors vs python split()[-n:])")
PY
