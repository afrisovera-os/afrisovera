#!/usr/bin/env bash
# Verify `rangewords <a> <b> <file>` prints words a..b (1-based, inclusive) of a VFS file, space-
# separated across all lines: the word-level parallel of rangelines. rangewords 1 3 about.txt ->
# first 3 words; rangewords 2 4 motd -> "to AfriSovera OS"; a missing file errors. Self-checking:
# python slices the split_whitespace word list from the seed content.
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
OUT="${TMPDIR:-/tmp}/rangewords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rangewords.png" \
    "echo warmup" "rangewords 1 3 about.txt" "rangewords 2 4 motd" \
    "rangewords 1 1 motd" "rangewords 1 3 nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rangewords:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

SEED = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                  "AI as an OS service. You own the device and the software on it."),
}

def message(name, a, b):  # mirror the kernel: words a..b (1-based inclusive), space-joined
    words = SEED[name].split()
    return "rangewords: " + " ".join(words[a-1:b])

for name, a, b in [("about.txt", 1, 3), ("motd", 2, 4), ("motd", 1, 1)]:
    want = message(name, a, b)   # every word DERIVED by python, never hand-typed
    if want not in s:
        print(f"RANGEWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: first-3 of about, an interior motd range, a single word, missing file
assert message("about.txt", 1, 3) in s
assert "rangewords: to AfriSovera OS" in s        # motd words 2..4
assert "rangewords: Welcome" in s                 # motd word 1
assert "rangewords: no such file: nope" in s

print("RANGEWORDS TEST PASSED  (about.txt 1..3, motd 2..4=to AfriSovera OS, motd 1..1=Welcome, "
      "missing-file errors vs python word-range slice)")
PY
