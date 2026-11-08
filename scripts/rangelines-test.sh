#!/usr/bin/env bash
# Verify `rangelines <a> <b> <file>` prints lines a..b (1-based, inclusive) of a VFS file, one
# per output line. An arbitrary line-range selector distinct from headlines/taillines/line/
# oddlines/evenlines. rangelines 1 2 about.txt -> both lines; 2 2 -> line2; 1 1 motd -> motd;
# a missing file errors. Self-checking: python derives each range line from the seed content.
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
OUT="${TMPDIR:-/tmp}/rangelines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rangelines.png" \
    "echo warmup" "rangelines 1 2 about.txt" "rangelines 2 2 about.txt" \
    "rangelines 1 1 motd" "rangelines 1 2 nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rangelines:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

SEED = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                  "AI as an OS service. You own the device and the software on it."),
}

def expected(name, a, b):  # mirror the kernel: lines a..b (1-based inclusive), else skipped
    lines = SEED[name].split("\n")
    return [f"rangelines: {lines[i]}" for i in range(len(lines)) if a <= i + 1 <= b]

for name, a, b in [("about.txt", 1, 2), ("about.txt", 2, 2), ("motd", 1, 1)]:
    for want in expected(name, a, b):   # every line DERIVED by python, never hand-typed
        if want not in s:
            print(f"RANGELINES TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: both about.txt lines, the single-line range, the missing-file error
assert f"rangelines: {SEED['about.txt'].split(chr(10))[0]}" in s   # line 1
assert f"rangelines: {SEED['about.txt'].split(chr(10))[1]}" in s   # line 2
assert f"rangelines: {SEED['motd']}" in s                          # motd (1..1)
assert "rangelines: no such file: nope" in s

print("RANGELINES TEST PASSED  (about.txt 1..2 both lines, 2..2 line2, motd 1..1, missing-file "
      "errors vs python line-range slice)")
PY
