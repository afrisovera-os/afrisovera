#!/usr/bin/env bash
# Verify `oddlines <file>` prints the odd-numbered lines (1st,3rd,... 1-based) of a VFS
# file, newline-joined. about.txt (2 lines) -> line 1 ONLY (line 2 excluded); motd (1
# line) -> its single line. A line-parity selector distinct from head/tail (byte) and
# headlines/taillines/line. Self-checking: python replicates split('\n')[::2].
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
OUT="${TMPDIR:-/tmp}/oddlines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/oddlines.png" \
    "echo warmup" "oddlines about.txt" "oddlines motd" "oddlines nope" "oddlines" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "oddlines:|usage: oddlines" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # capture renders embedded \n as \r\r\n

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
L2 = ABOUT.split("\n")[1]

def odd(text):  # mirror the kernel: keep 0-based even indices == 1-based odd lines
    return "\n".join(l for i, l in enumerate(text.split("\n")) if i % 2 == 0)

for text in [ABOUT, MOTD]:
    want = f"oddlines: {odd(text)}"
    if want not in s:
        print(f"ODDLINES TEST FAILED (missing '{want}')"); sys.exit(1)

# line 2 of about.txt is EVEN -> must never appear in any oddlines output
assert L2 not in s, "even line 2 leaked into oddlines output"
# error branches
assert "oddlines: no such file: nope" in s, "missing not-found branch"
assert "usage: oddlines <file>" in s, "missing usage branch"

print("ODDLINES TEST PASSED  (about.txt->line1 only (line2 excluded), motd->its line, "
      "nope->not found, usage OK vs python odd-line selection)")
PY
