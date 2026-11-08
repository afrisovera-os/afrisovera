#!/usr/bin/env bash
# Verify `firstsentencef <file>` prints the FIRST sentence of a VFS FILE (file analogue of
# firstsentence): motd -> "Welcome to AfriSovera OS -- a sovereign, capability-secured OS"
# (its only sentence), about.txt -> "AfriSovera: one codebase, WASM-native apps, no gatekeeper
# store,\nAI as an OS service" (crosses its embedded line break), a miss -> "no such file".
# Same . ! ? terminator rule as firstsentence/countsentencesf. Self-checking against the known
# seed-file content.
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
OUT="${TMPDIR:-/tmp}/firstsentencef.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=4 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstsentencef.png" \
    "echo warmup" "firstsentencef motd" "firstsentencef about.txt" \
    "firstsentencef nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstsentencef:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # serial renders embedded \n as \r\r\n -> strip all \r

# Known seed content (kernel/src/vfs.rs) -> DERIVED first sentence via the shared segmentation rule.
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def firstsentence(text):
    seg_start = 0
    has_content = False
    first = None
    for i, ch in enumerate(text):
        if ch in ".!?":
            if has_content:
                first = text[seg_start:i].strip()
                break
            seg_start = i + 1
        elif not ch.isspace():
            has_content = True
    if first is None and has_content:
        first = text[seg_start:].strip()
    return first

exp_motd = f"firstsentencef: motd = {firstsentence(motd)}"
exp_about = f"firstsentencef: about.txt = {firstsentence(about)}"
for w in (exp_motd, exp_about):
    if w not in s:
        print(f"FIRSTSENTENCEF TEST FAILED (missing '{w}')"); sys.exit(1)
if "firstsentencef: no such file: nope" not in s:
    print("FIRSTSENTENCEF TEST FAILED (missing no-such-file branch)"); sys.exit(1)

print(f"FIRSTSENTENCEF TEST PASSED  (motd->{firstsentence(motd)!r}, "
      f"about.txt->{firstsentence(about)!r}, nope->no such file; VFS file analogue of firstsentence)")
PY
