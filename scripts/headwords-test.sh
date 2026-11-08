#!/usr/bin/env bash
# Verify `headwords <n> <file>` prints the FIRST n whitespace-delimited words of a VFS file
# (all of them if fewer than n): about.txt 3 -> "AfriSovera: one codebase,", about.txt 1 ->
# "AfriSovera:", motd 2 -> "Welcome to", motd 100 -> whole file (10 words), missing file errors.
# Word-head mirror of headlines and the open-ended complement of tailwords. Self-checking:
# python's content.split()[:n] is the ref.
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
OUT="${TMPDIR:-/tmp}/headwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/headwords.png" \
    "echo warmup" "headwords 3 about.txt" "headwords 1 about.txt" \
    "headwords 2 motd" "headwords 100 motd" "headwords 5 nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "headwords:|usage: headwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# Exact seed content from vfs::seed() (kernel/src/vfs.rs).
MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def head(content, n):
    return "headwords: " + " ".join(content.split()[:n] if n else [])   # DERIVED via python

checks = [head(ABOUT, 3), head(ABOUT, 1), head(MOTD, 2), head(MOTD, 100)]
for want in checks:
    if want not in s:
        print(f"HEADWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: first-3, first-1, first-2, n>count (whole file)
assert "headwords: AfriSovera: one codebase," in s
assert "headwords: AfriSovera:" in s
assert "headwords: Welcome to" in s
assert "headwords: Welcome to AfriSovera OS -- a sovereign, capability-secured OS." in s
assert "headwords: no such file: nope.txt" in s

print("HEADWORDS TEST PASSED  (about.txt 3->AfriSovera: one codebase,, 1->AfriSovera:, "
      "motd 2->Welcome to, motd 100->whole file, nope.txt errors vs python split()[:n])")
PY
