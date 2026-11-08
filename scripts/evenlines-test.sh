#!/usr/bin/env bash
# Verify `evenlines <file>` prints the even-numbered lines (2nd,4th,... 1-based) of a VFS
# file -- the complement of oddlines. about.txt (2 lines) -> line 2 ONLY (line 1 excluded);
# motd (1 line) -> nothing (no even line). A line-parity selector. Self-checking: python
# replicates split('\n') keeping 0-based odd indices.
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
OUT="${TMPDIR:-/tmp}/evenlines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/evenlines.png" \
    "echo warmup" "evenlines about.txt" "evenlines motd" "evenlines nope" "evenlines" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "evenlines:|usage: evenlines" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # capture renders embedded \n as \r\r\n

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
L1 = ABOUT.split("\n")[0]
L2 = ABOUT.split("\n")[1]

# about.txt evenlines -> line 2 only
assert f"evenlines: {L2}" in s, "missing about.txt line 2"
# line 1 of about.txt is ODD -> must never appear in any evenlines output
assert L1 not in s, "odd line 1 leaked into evenlines output"
# motd is a single ODD line -> evenlines emits nothing, so the motd text must be absent
assert MOTD not in s, "motd (odd single line) leaked into evenlines output"
# error branches
assert "evenlines: no such file: nope" in s, "missing not-found branch"
assert "usage: evenlines <file>" in s, "missing usage branch"

# derive-and-check the exact reference for about.txt
def even(text):
    return "\n".join(l for i, l in enumerate(text.split("\n")) if i % 2 == 1)
assert f"evenlines: {even(ABOUT)}" in s

print("EVENLINES TEST PASSED  (about.txt->line2 only (line1 excluded), motd->empty (odd "
      "line excluded), nope->not found, usage OK vs python even-line selection)")
PY
