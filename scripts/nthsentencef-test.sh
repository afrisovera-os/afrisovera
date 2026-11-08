#!/usr/bin/env bash
# Verify `nthsentencef <n> <file>` prints the Nth (1-based) sentence of a VFS FILE (file
# analogue of nthsentence): motd sentence 1 -> "Welcome to AfriSovera OS -- a sovereign,
# capability-secured OS" (its only sentence), about.txt sentence 2 -> "You own the device
# and the software on it" (crosses its embedded line break), about.txt sentence 3 -> "fewer
# than" (out of range), a missing file -> "no such file". Same . ! ? terminator rule as
# nthsentence/firstsentencef/lastsentencef/countsentencesf. Self-checking against the known
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
OUT="${TMPDIR:-/tmp}/nthsentencef.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthsentencef.png" \
    "echo warmup" "nthsentencef 1 motd" "nthsentencef 2 about.txt" \
    "nthsentencef 3 about.txt" "nthsentencef 1 nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthsentencef:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # serial renders embedded \n as \r\r\n -> strip all \r

# Known seed content (kernel/src/vfs.rs) -> DERIVED Nth sentence via the shared segmentation rule.
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def nthsentence(text, n):
    seg_start = 0
    has_content = False
    count = 0
    found = None
    for i, ch in enumerate(text):
        if ch in ".!?":
            if has_content:
                count += 1
                if count == n:
                    found = text[seg_start:i].strip()
                has_content = False
            seg_start = i + 1
        elif not ch.isspace():
            has_content = True
    if has_content:
        count += 1
        if count == n:
            found = text[seg_start:].strip()
    return found

exp_motd = f"nthsentencef: motd sentence 1 = {nthsentence(motd, 1)}"
exp_about2 = f"nthsentencef: about.txt sentence 2 = {nthsentence(about, 2)}"
for w in (exp_motd, exp_about2):
    if w not in s:
        print(f"NTHSENTENCEF TEST FAILED (missing '{w}')"); sys.exit(1)
if "nthsentencef: about.txt has fewer than 3 sentence(s)" not in s:
    print("NTHSENTENCEF TEST FAILED (missing out-of-range branch)"); sys.exit(1)
if "nthsentencef: no such file: nope" not in s:
    print("NTHSENTENCEF TEST FAILED (missing no-such-file branch)"); sys.exit(1)

print(f"NTHSENTENCEF TEST PASSED  (motd sentence 1->{nthsentence(motd, 1)!r}, "
      f"about.txt sentence 2->{nthsentence(about, 2)!r}, sentence 3->fewer-than, "
      f"nope->no such file; VFS file analogue of nthsentence)")
PY
