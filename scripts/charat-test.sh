#!/usr/bin/env bash
# Verify the `charat <n> <file>` command prints the Nth (1-based) byte of a VFS
# file's whole content, reporting the character. Counts every byte (newlines
# included), unlike line (Nth line), wordn (Nth word), cut (a field), and
# linelen (a line's length). Self-checking: python re-derives s[n-1] from the
# known seed contents; out-of-range and missing-file paths are pinned.
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
OUT="${TMPDIR:-/tmp}/charat.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/charat.png" \
    "echo warmup" "charat 1 motd" "charat 12 motd" "charat 20 motd" \
    "charat 1 about.txt" "charat 64 motd" "charat 1 nope" "charat motd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "charat:|usage: charat" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed contents from kernel/src/vfs.rs seed().
seed = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

# in-range char positions -> the exact character
for name, n in [("motd", 1), ("motd", 12), ("motd", 20), ("about.txt", 1)]:
    want = f"charat: char {n} is {seed[name][n-1]}"
    if want not in s:
        print(f"CHARAT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly
assert "charat: char 1 is W" in s      # motd[0]
assert "charat: char 12 is A" in s      # motd[11] -- the 'A' of AfriSovera
assert "charat: char 20 is r" in s      # motd[19]
assert "charat: char 1 is A" in s       # about.txt[0]

# out of range: motd is 63 chars, so char 64 must report the shortfall (not a char)
if "charat: file has fewer than 64 chars" not in s:
    print("CHARAT TEST FAILED (out-of-range n should report shortfall)"); sys.exit(1)
# missing file + no-arg
if "charat: no such file: nope" not in s:
    print("CHARAT TEST FAILED (missing-file path)"); sys.exit(1)
if "usage: charat <n> <file>" not in s:
    print("CHARAT TEST FAILED (no-arg usage line)"); sys.exit(1)

print("CHARAT TEST PASSED  (motd 1/12/20 -> W/A/r, about.txt 1 -> A vs python; "
      "n=64 -> shortfall; missing-file + no-arg handled)")
PY
