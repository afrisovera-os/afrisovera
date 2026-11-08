#!/usr/bin/env bash
# Verify the `firstchars <file>` command concatenates the first character of each
# (non-empty) line of a VFS file into one string (about.txt -> "AA", motd -> "W").
# The line-level analogue of initials (first char of each word); distinct from
# firstword (first word of each line). Self-checking: python derives from the seed.
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
OUT="${TMPDIR:-/tmp}/firstchars.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# about.txt has TWO lines (both start with 'A') -> "AA"; motd is one line; a miss.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstchars.png" \
    "echo warmup" "firstchars about.txt" "firstchars motd" "firstchars nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstchars:|usage: firstchars" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed file contents (from kernel/src/vfs.rs seed()).
FILES = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def firstchars(fname):
    return "".join(line[0] for line in FILES[fname].splitlines() if line)  # DERIVED

for fname in ["about.txt", "motd"]:
    want = f"firstchars: {firstchars(fname)}"
    if want not in s:
        print(f"FIRSTCHARS TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: about.txt's two lines both start with 'A' -> "AA"; motd -> "W"
assert "firstchars: AA" in s
assert "firstchars: W" in s
assert "firstchars: no such file: nope.txt" in s

print("FIRSTCHARS TEST PASSED  (about.txt->AA, motd->W, missing-file handled vs python; "
      "first char of each line concatenated)")
PY
