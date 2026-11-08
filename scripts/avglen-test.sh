#!/usr/bin/env bash
# Verify the `avglen` command reports the average line length (line bytes / line
# count, excluding the '\n') of a VFS file -- a derived statistic wc does not show.
# The harness computes the expected numbers from the KNOWN seeded content
# (about.txt = two known constant lines; motd = a known constant line), so it is
# self-checking. Also checks the no-such-file and usage paths.
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
OUT="${TMPDIR:-/tmp}/avglen.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/avglen.png" \
    "avglen about.txt" "avglen motd" "avglen nope.txt" "avglen" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "avglen:|usage: avglen" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# Known seeded content (lines exclude the '\n', matching s.lines()).
l1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
l2 = "AI as an OS service. You own the device and the software on it."
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

ab_bytes = len(l1) + len(l2)
want_about = f"avglen: 2 lines, {ab_bytes} bytes, average {ab_bytes // 2} bytes/line"
want_motd = f"avglen: 1 lines, {len(motd)} bytes, average {len(motd)} bytes/line"

for want in (want_about, want_motd):
    if want not in s:
        print(f"AVGLEN TEST FAILED (missing '{want}')"); sys.exit(1)

if "avglen: no such file: nope.txt" not in s:
    print("AVGLEN TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: avglen <file>" not in s:
    print("AVGLEN TEST FAILED (missing usage line)"); sys.exit(1)

print(f"AVGLEN TEST PASSED  (about.txt 2 lines/{ab_bytes}B/avg {ab_bytes//2}, motd 1 line/{len(motd)}B; no-such-file + usage)")
PY
