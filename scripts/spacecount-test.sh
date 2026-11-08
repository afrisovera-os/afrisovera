#!/usr/bin/env bash
# Verify the `spacecount <file>` command counts the literal space characters (0x20) in a
# VFS file. motd has 8 (9 words -> 8 gaps); about.txt has 20 (line1 7 gaps + line2 13
# gaps, the newline is not a space). Char-class companion of punctcount/vowelcount;
# distinct from charcount/wc/despace. Self-checking: python counts ' ' in the seed text.
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
OUT="${TMPDIR:-/tmp}/spacecount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/spacecount.png" \
    "echo warmup" "spacecount motd" "spacecount about.txt" "spacecount nope" "spacecount" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "spacecount:|usage: spacecount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

SEEDS = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\nAI as an OS service. You own the device and the software on it.",
}

for name, text in SEEDS.items():
    want = f"spacecount: {name} has {text.count(' ')} space characters"
    if want not in s:
        print(f"SPACECOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# error branches
assert "spacecount: no such file: nope" in s, "missing no-such-file branch"
assert "usage: spacecount <file>" in s, "missing usage branch"

# pin the two seed counts explicitly (derived above)
assert "spacecount: motd has 8 space characters" in s
assert "spacecount: about.txt has 20 space characters" in s

print("SPACECOUNT TEST PASSED  (motd->8, about.txt->20 vs python ' ' count; "
      "no-such-file + usage branches OK)")
PY
