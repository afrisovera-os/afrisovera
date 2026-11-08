#!/usr/bin/env bash
# Verify the `headlines <n> <file>` command prints the first N LINES of a VFS file (whole
# file if it has fewer). about.txt has 2 lines: headlines 1 -> line1 only, headlines 2 ->
# both. motd is 1 line. The line-oriented companion of head/tail (byte-based); distinct
# from `line` (one line by index). Self-checking: python replicates split('\n')[:n].
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
OUT="${TMPDIR:-/tmp}/headlines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/headlines.png" \
    "echo warmup" "headlines 1 about.txt" "headlines 2 about.txt" "headlines 1 motd" "headlines 3 nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "headlines:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
# the serial capture renders an embedded '\n' as '\r\r\n'; strip CRs so a multi-line
# body compares as a plain '\n'-joined string (the kernel output itself is correct).
s = raw.replace("\r", "")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
L1 = ABOUT.split("\n")[0]
L2 = ABOUT.split("\n")[1]

# missing file branch
assert "headlines: nope not found" in s, "missing not-found branch"

# derive-and-check: python computes the first-n-lines join for each case
def hl(text, n):
    return "\n".join(text.split("\n")[:n])
for text, n in [(ABOUT, 1), (ABOUT, 2), (MOTD, 1)]:
    want = f"headlines: {hl(text, n)}"
    if want not in s:
        print(f"HEADLINES TEST FAILED (missing '{want}')"); sys.exit(1)

# headlines 2 puts line 2 directly after line 1 (whole file); headlines 1 does not
assert f"headlines: {L1}\n{L2}" in s, "headlines 2 should join both lines"

print("HEADLINES TEST PASSED  (about.txt 1->line1, 2->both lines, motd 1->whole line, "
      "nope->not found vs python split-lines)")
PY
