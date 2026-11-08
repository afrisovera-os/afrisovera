#!/usr/bin/env bash
# Verify the `lastchars <file>` command concatenates the last character of each
# (non-empty) line of a VFS file into one string (about.txt -> ",.", motd -> ".").
# The mirror of firstchars; distinct from lastword (last word of each line).
# Self-checking: python derives from the exact seed content.
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
OUT="${TMPDIR:-/tmp}/lastchars.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# about.txt: line1 ends ',' line2 ends '.' -> ",."; motd ends '.'; a miss.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastchars.png" \
    "echo warmup" "lastchars about.txt" "lastchars motd" "lastchars nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastchars:|usage: lastchars" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed file contents (from kernel/src/vfs.rs seed()).
FILES = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def lastchars(fname):
    return "".join(line[-1] for line in FILES[fname].splitlines() if line)  # DERIVED

for fname in ["about.txt", "motd"]:
    want = f"lastchars: {lastchars(fname)}"
    if want not in s:
        print(f"LASTCHARS TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: about.txt line1 ends ',' and line2 ends '.' -> ",."; motd ends '.'
assert "lastchars: ,." in s
assert "lastchars: ." in s
assert "lastchars: no such file: nope.txt" in s

print("LASTCHARS TEST PASSED  (about.txt->,. , motd->. , missing-file handled vs python; "
      "last char of each line concatenated)")
PY
