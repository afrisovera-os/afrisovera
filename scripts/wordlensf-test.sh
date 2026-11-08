#!/usr/bin/env bash
# Verify `wordlensf <file>` lists the byte length of each whitespace-delimited word of a VFS
# file (counting across all lines), space-separated: the file analogue of wordlengths (inline
# text). motd -> "7 2 10 2 2 1 10 18 3"; about.txt -> the 22 per-word lengths. Self-checking:
# python derives each length from the known seed content, never hand-typed.
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
OUT="${TMPDIR:-/tmp}/wordlensf.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordlensf.png" \
    "echo warmup" "wordlensf motd" "wordlensf about.txt" "wordlensf nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordlensf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

SEED = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                  "AI as an OS service. You own the device and the software on it."),
}

def message(name):  # mirror the kernel: byte length of each split_whitespace word
    lens = " ".join(str(len(w.encode("latin1"))) for w in SEED[name].split())
    return f"wordlensf: {lens}"

for name in ("motd", "about.txt"):
    want = message(name)   # every length DERIVED by python, never hand-typed
    if want not in s:
        print(f"WORDLENSF TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: motd's 9 lengths, about's 22-word list, missing-file error
assert message("motd") in s
assert message("about.txt") in s
assert "wordlensf: no such file: nope" in s

# sanity: motd has 9 words, about.txt has 22 words (word count == number of length tokens)
assert len(message("motd").split(": ",1)[1].split()) == 9
assert len(message("about.txt").split(": ",1)[1].split()) == 22

print("WORDLENSF TEST PASSED  (motd 9 word-lengths, about.txt 22 word-lengths, missing-file "
      "errors vs python per-word byte lengths)")
PY
