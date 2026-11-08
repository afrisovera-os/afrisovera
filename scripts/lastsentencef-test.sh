#!/usr/bin/env bash
# Verify `lastsentencef <file>` prints the LAST sentence of a VFS FILE (file analogue of
# lastsentence): motd -> "Welcome to AfriSovera OS -- a sovereign, capability-secured OS"
# (its only sentence), about.txt -> "You own the device and the software on it" (its
# second/last sentence), a miss -> "no such file". Same . ! ? terminator rule as
# lastsentence/firstsentencef/countsentencesf. Self-checking against the known seed-file
# content.
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
OUT="${TMPDIR:-/tmp}/lastsentencef.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=4 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastsentencef.png" \
    "echo warmup" "lastsentencef motd" "lastsentencef about.txt" \
    "lastsentencef nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastsentencef:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # serial renders embedded \n as \r\r\n -> strip all \r

# Known seed content (kernel/src/vfs.rs) -> DERIVED last sentence via the shared segmentation rule.
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def lastsentence(text):
    seg_start = 0
    has_content = False
    last = None
    for i, ch in enumerate(text):
        if ch in ".!?":
            if has_content:
                last = text[seg_start:i].strip()
                has_content = False
            seg_start = i + 1
        elif not ch.isspace():
            has_content = True
    if has_content:
        last = text[seg_start:].strip()
    return last

exp_motd = f"lastsentencef: motd = {lastsentence(motd)}"
exp_about = f"lastsentencef: about.txt = {lastsentence(about)}"
for w in (exp_motd, exp_about):
    if w not in s:
        print(f"LASTSENTENCEF TEST FAILED (missing '{w}')"); sys.exit(1)
if "lastsentencef: no such file: nope" not in s:
    print("LASTSENTENCEF TEST FAILED (missing no-such-file branch)"); sys.exit(1)

print(f"LASTSENTENCEF TEST PASSED  (motd->{lastsentence(motd)!r}, "
      f"about.txt->{lastsentence(about)!r}, nope->no such file; VFS file analogue of lastsentence)")
PY
