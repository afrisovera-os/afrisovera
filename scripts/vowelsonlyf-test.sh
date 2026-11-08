#!/usr/bin/env bash
# Verify `vowelsonlyf <file>` keeps ONLY the ASCII vowels of a whole VFS file, dropping everything
# else INCLUDING newlines (so a multi-line file collapses to one run of vowels) -- the file analogue
# of vowelsonly and the complement of disemvowelf. Tests motd and about.txt (both lines' vowels
# concatenated), plus a missing file. Self-checking: python keeps aeiouAEIOU.
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
OUT="${TMPDIR:-/tmp}/vowelsonlyf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/vowelsonlyf.png" \
    "echo warmup" "vowelsonlyf motd" "vowelsonlyf about.txt" "vowelsonlyf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vowelsonlyf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
# about.txt is two lines; newlines are dropped so the vowels of both lines concatenate.
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
         "AI as an OS service. You own the device and the software on it.")

def vowels_only(t):                  # DERIVED reference
    return "".join(c for c in t if c in "aeiouAEIOU")

want_motd = "vowelsonlyf: " + vowels_only(MOTD)
if want_motd not in s:
    print(f"VOWELSONLYF TEST FAILED (missing motd: {want_motd!r})"); sys.exit(1)

want_about = "vowelsonlyf: " + vowels_only(ABOUT)
if want_about not in s:
    print(f"VOWELSONLYF TEST FAILED (missing about: {want_about!r})"); sys.exit(1)

assert "vowelsonlyf: no such file: nope.txt" in s
print("VOWELSONLYF TEST PASSED  (motd + collapsed about.txt vowels vs python aeiou; nope.txt errors)")
PY
