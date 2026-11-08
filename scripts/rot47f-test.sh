#!/usr/bin/env bash
# Verify `rot47f <file>` applies ROT47 (rotate printable ASCII 33..126 by 47, other bytes passthrough)
# over a whole VFS file -- the file analogue of rot47, self-inverse. Tests motd and about.txt (both
# lines), plus a missing file. Self-checking: python ROT47 reference.
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
OUT="${TMPDIR:-/tmp}/rot47f.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rot47f.png" \
    "echo warmup" "rot47f motd" "rot47f about.txt" "rot47f nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rot47f:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT_L1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
ABOUT_L2 = "AI as an OS service. You own the device and the software on it."

def rot47(t):                        # DERIVED reference
    return "".join(chr(33 + (ord(c) - 33 + 47) % 94) if 33 <= ord(c) <= 126 else c for c in t)

want_motd = "rot47f: " + rot47(MOTD)
if want_motd not in s:
    print(f"ROT47F TEST FAILED (missing motd: {want_motd!r})"); sys.exit(1)

for ln in (ABOUT_L1, ABOUT_L2):      # two-line file: check each line as a substring
    if rot47(ln) not in s:
        print(f"ROT47F TEST FAILED (missing about line: {rot47(ln)!r})"); sys.exit(1)

# self-inverse sanity
assert rot47(rot47(MOTD)) == MOTD
assert "rot47f: no such file: nope.txt" in s
print("ROT47F TEST PASSED  (motd + about.txt both lines vs python ROT47; self-inverse; nope.txt errors)")
PY
