#!/usr/bin/env bash
# Verify the `punctcount <file>` command counts the ASCII PUNCTUATION characters in a
# VFS file (the 32-symbol set !"#$%&'()*+,-./:;<=>?@[\]^_`{|}~). motd has 5 (-- , - .),
# about.txt has 7 (: , - , , . .). The char-class companion of vowelcount; distinct from
# charcount (one char) and wc. Self-checking: python replicates Rust's is_ascii_punctuation.
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
OUT="${TMPDIR:-/tmp}/punctcount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/punctcount.png" \
    "echo warmup" "punctcount motd" "punctcount about.txt" "punctcount nope" "punctcount" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "punctcount:|usage: punctcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

SEEDS = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\nAI as an OS service. You own the device and the software on it.",
}

def punct(t):  # matches Rust u8::is_ascii_punctuation
    return sum(1 for c in t if (0x21 <= ord(c) <= 0x2F) or (0x3A <= ord(c) <= 0x40)
               or (0x5B <= ord(c) <= 0x60) or (0x7B <= ord(c) <= 0x7E))

for name, text in SEEDS.items():
    want = f"punctcount: {name} has {punct(text)} punctuation characters"
    if want not in s:
        print(f"PUNCTCOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# error branches
assert "punctcount: no such file: nope" in s, "missing no-such-file branch"
assert "usage: punctcount <file>" in s, "missing usage branch"

# pin the two seed counts explicitly (derived above, shown here for the record)
assert "punctcount: motd has 5 punctuation characters" in s
assert "punctcount: about.txt has 7 punctuation characters" in s

print("PUNCTCOUNT TEST PASSED  (motd->5, about.txt->7 vs python is_ascii_punctuation; "
      "no-such-file + usage branches OK)")
PY
