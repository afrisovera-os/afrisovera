#!/usr/bin/env bash
# Verify the `taillines <n> <file>` command prints the last N LINES of a VFS file (whole
# file if it has fewer). about.txt has 2 lines: taillines 1 -> line2 only, taillines 2 ->
# both. motd is 1 line. The line-oriented mirror of headlines, tail analogue of head/tail
# (byte-based). Self-checking: python replicates split('\n')[len-n:].
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
OUT="${TMPDIR:-/tmp}/taillines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/taillines.png" \
    "echo warmup" "taillines 1 about.txt" "taillines 2 about.txt" "taillines 1 motd" "taillines 3 nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "taillines:" "$OUT" | sed 's/^/  /'

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
assert "taillines: nope not found" in s, "missing not-found branch"

def tl(text, n):
    lines = text.split("\n")
    return "\n".join(lines[max(0, len(lines) - n):])
for text, n in [(ABOUT, 1), (ABOUT, 2), (MOTD, 1)]:
    want = f"taillines: {tl(text, n)}"
    if want not in s:
        print(f"TAILLINES TEST FAILED (missing '{want}')"); sys.exit(1)

# taillines 1 about.txt -> ONLY line 2 (line 1 must NOT immediately precede it in that tag)
assert f"taillines: {L2}" in s, "taillines 1 should be line 2 only"
# taillines 2 about.txt -> both lines, line1 then line2
assert f"taillines: {L1}\n{L2}" in s, "taillines 2 should join both lines"

print("TAILLINES TEST PASSED  (about.txt 1->line2, 2->both lines, motd 1->whole line, "
      "nope->not found vs python split-lines tail)")
PY
