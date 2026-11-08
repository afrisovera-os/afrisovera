#!/usr/bin/env bash
# Verify `rotlines <n> <file>` circular LEFT-rotates the LINES of a VFS file by n (line-level
# analogue of rotate/rotwords): about.txt has 2 lines, rotlines 1 about.txt swaps them,
# rotlines 2 about.txt (n == count) is identity (mod wrap). motd is 1 line, so any n is
# identity. Distinct from tac (REVERSES line order rather than shifting it). Self-checking:
# python replicates lines[k:] + lines[:k] with k = n % len(lines).
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
OUT="${TMPDIR:-/tmp}/rotlines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotlines.png" \
    "echo warmup" "rotlines 1 about.txt" "rotlines 2 about.txt" "rotlines 3 motd" "rotlines 1 nope" "rotlines about.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotlines:|usage: rotlines" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
# the serial capture renders an embedded '\n' as '\r\r\n'; strip CRs so a multi-line
# body compares as a plain '\n'-joined string (the kernel output itself is correct).
s = raw.replace("\r", "")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rl(text, n):  # mirror the kernel: circular left-rotate the LINES by n mod count
    lines = text.split("\n")
    k = n % len(lines)
    return "\n".join(lines[k:] + lines[:k])

for text, n in [(ABOUT, 1), (ABOUT, 2), (MOTD, 3)]:
    want = f"rotlines: {rl(text, n)}"
    if want not in s:
        print(f"ROTLINES TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the swap explicitly: rotlines 1 about.txt -> line2 then line1
L1, L2 = ABOUT.split("\n")
assert f"rotlines: {L2}\n{L1}" in s, "rotlines 1 should swap the two lines"
# n == line count is identity (mod wrap)
assert f"rotlines: {ABOUT}" in s, "rotlines 2 (== count) should be identity"
# single-line file: any n is identity
assert f"rotlines: {MOTD}" in s, "rotlines on a 1-line file should be identity for any n"

# error branches
assert "rotlines: nope not found" in s, "missing not-found branch"
assert "usage: rotlines <n> <file>" in s, "missing usage branch"

print("ROTLINES TEST PASSED  (about.txt 1->swapped, 2->identity (mod count), motd 3->identity, "
      "nope->not found, usage OK vs python)")
PY
