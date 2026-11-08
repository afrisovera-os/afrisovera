#!/usr/bin/env bash
# Verify `linefromend <n> <file>` prints the Nth line from the END of a VFS file: about.txt (2 lines)
# linefromend 1 -> line2, linefromend 2 -> line1, linefromend 3 -> "fewer than 3"; motd (1 line)
# linefromend 1 -> the motd, linefromend 2 -> "fewer than 2". Line-level mirror of line (Nth from start).
# Self-checking against the known seeded file content.
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
OUT="${TMPDIR:-/tmp}/linefromend.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/linefromend.png" \
    "echo warmup" "linefromend 1 about.txt" "linefromend 2 about.txt" "linefromend 3 about.txt" \
    "linefromend 1 motd" "linefromend 2 motd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "AfriSovera|AI as an OS|Welcome to|fewer than" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# Known seeded content (lines exclude the '\n'), matching s.lines().
l1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
l2 = "AI as an OS service. You own the device and the software on it."
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

# linefromend 1 about -> l2 (last), 2 -> l1, 3 -> fewer; motd 1 -> motd, 2 -> fewer
if l2 not in s:  # last line of about.txt
    print("LINEFROMEND TEST FAILED (missing last line l2)"); sys.exit(1)
if l1 not in s:  # second-to-last line of about.txt
    print("LINEFROMEND TEST FAILED (missing line l1)"); sys.exit(1)
if motd not in s:
    print("LINEFROMEND TEST FAILED (missing motd)"); sys.exit(1)
if "linefromend: file has fewer than 3 lines" not in s:
    print("LINEFROMEND TEST FAILED (missing about fewer-than-3)"); sys.exit(1)
if "linefromend: file has fewer than 2 lines" not in s:
    print("LINEFROMEND TEST FAILED (missing motd fewer-than-2)"); sys.exit(1)

print("LINEFROMEND TEST PASSED  (about.txt 1->last line, 2->first line, 3->fewer; motd 1->motd, "
      "2->fewer; line-level mirror of line)")
PY
