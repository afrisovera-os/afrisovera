#!/usr/bin/env bash
# Verify `rotatef <n> <file>` circular LEFT-rotates the raw BYTES of a whole VFS file by n
# (file analogue of rotate/inline, distinct from rotlines which rotates whole LINES not raw
# bytes -- embedded newlines are just data here so line boundaries generally shift/merge).
# motd is 63 bytes: rotatef 1 motd drops the leading 'W' to the end. about.txt is 128 bytes
# (64+1+63, embedded \n counted): rotatef 128 about.txt (n == len) is identity (mod wrap).
# Self-checking: python replicates bytes[k:] + bytes[:k] with k = n % len(bytes).
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
OUT="${TMPDIR:-/tmp}/rotatef.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotatef.png" \
    "echo warmup" "rotatef 1 motd" "rotatef 128 about.txt" "rotatef 1 nope" "rotatef motd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotatef:|usage: rotatef" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
# the serial capture renders an embedded '\n' as '\r\r\n'; strip CRs so a multi-line
# body compares as a plain '\n'-joined string (the kernel output itself is correct).
s = raw.replace("\r", "")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rf(text, n):  # mirror the kernel: circular left-rotate the raw BYTES by n mod len
    b = text.encode("latin1")
    k = n % len(b)
    return (b[k:] + b[:k]).decode("latin1")

for text, n in [(MOTD, 1), (ABOUT, 128)]:
    want = f"rotatef: {rf(text, n)}"
    if want not in s:
        print(f"ROTATEF TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the byte-shift explicitly: rotatef 1 motd drops the leading 'W' to the end
assert "rotatef: elcome to AfriSovera OS -- a sovereign, capability-secured OS.W" in s, \
    "rotatef 1 motd should shift the leading byte to the end"
# n == byte length is identity (mod wrap)
assert f"rotatef: {ABOUT}" in s, "rotatef 128 (== len) should be identity"

# error branches
assert "rotatef: nope not found" in s, "missing not-found branch"
assert "usage: rotatef <n> <file>" in s, "missing usage branch"

print("ROTATEF TEST PASSED  (motd 1->byte-shifted, about.txt 128->identity (mod len), "
      "nope->not found, usage OK vs python)")
PY
