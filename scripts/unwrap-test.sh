#!/usr/bin/env bash
# Verify `unwrap <file>` joins ALL lines of a VFS file into ONE line separated by single
# spaces (inverse of wrap): about.txt (2 lines) -> both lines joined by a space; motd (1
# line) -> unchanged; a missing file -> "no such file". Distinct from tac/rwordsf/revf.
# Self-checking: python joins the known seed content's lines and compares to the output.
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
OUT="${TMPDIR:-/tmp}/unwrap.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/unwrap.png" \
    "echo warmup" "unwrap about.txt" "unwrap motd" "unwrap nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unwrap:|usage: unwrap" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed content written by vfs::seed() (kernel/src/vfs.rs). The unwrap of a file
# is its lines joined by one space -- DERIVED here, never hand-typed on the expected side.
MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def unwrap(content):
    return "unwrap: " + " ".join(content.split("\n"))

want_about = unwrap(ABOUT)
want_motd  = unwrap(MOTD)   # single line -> unchanged
for want in (want_about, want_motd):
    if want not in s:
        print(f"UNWRAP TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: multi-line join, single-line passthrough, missing file
assert "unwrap: AfriSovera: one codebase, WASM-native apps, no gatekeeper store, AI as an OS service. You own the device and the software on it." in s
assert "unwrap: Welcome to AfriSovera OS -- a sovereign, capability-secured OS." in s
assert "unwrap: no such file: nope.txt" in s

print("UNWRAP TEST PASSED  (about.txt 2 lines joined by space, motd 1 line unchanged, "
      "nope.txt reports no-such-file vs python line-join)")
PY
