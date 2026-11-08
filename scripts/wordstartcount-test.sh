#!/usr/bin/env bash
# Verify the `wordstartcount <ch> <file>` command counts how many whitespace-
# delimited words in a VFS file BEGIN with a given character (first char of
# <ch>, case-sensitive). Distinct from charcount (a char anywhere), grepc
# (matching lines), wordfreq (most frequent word), countword (inline). Self-
# checking: python re-derives the counts from the known seed contents.
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
OUT="${TMPDIR:-/tmp}/wordstartcount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordstartcount.png" \
    "echo warmup" "wordstartcount O motd" "wordstartcount a motd" \
    "wordstartcount t motd" "wordstartcount a about.txt" "wordstartcount o about.txt" \
    "wordstartcount O nope" "wordstartcount O" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordstartcount:|usage: wordstartcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed contents from kernel/src/vfs.rs seed().
seed = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def wsc(text, ch):
    return sum(1 for w in text.split() if w and w[0] == ch)

cases = [("motd", "O"), ("motd", "a"), ("motd", "t"),
         ("about.txt", "a"), ("about.txt", "o")]
for name, ch in cases:
    want = f"wordstartcount: {wsc(seed[name], ch)} words start with {ch}"
    if want not in s:
        print(f"WORDSTARTCOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly (case-sensitive: 'a' != 'A')
assert "wordstartcount: 2 words start with O" in s   # motd: OS, OS.
assert "wordstartcount: 1 words start with a" in s   # motd: 'a' (AfriSovera is 'A')
assert "wordstartcount: 4 words start with a" in s   # about.txt: apps, as, an, and
assert "wordstartcount: 3 words start with o" in s   # about.txt: one, own, on

# missing file + no-arg
if "wordstartcount: no such file: nope" not in s:
    print("WORDSTARTCOUNT TEST FAILED (missing-file path)"); sys.exit(1)
if "usage: wordstartcount <ch> <file>" not in s:
    print("WORDSTARTCOUNT TEST FAILED (no-arg usage line)"); sys.exit(1)

print("WORDSTARTCOUNT TEST PASSED  (motd O/a/t->2/1/1, about.txt a/o->4/3 vs python; "
      "case-sensitive; missing-file + no-arg handled)")
PY
