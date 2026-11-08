#!/usr/bin/env bash
# Verify the `wordendcount <ch> <file>` command counts how many whitespace-
# delimited words in a VFS file END with a given character (first char of <ch>,
# case-sensitive) -- the mirror of wordstartcount. Distinct from charcount (a
# char anywhere), grepc (matching lines), wordfreq, countword. Self-checking:
# python re-derives the counts from the known seed contents.
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
OUT="${TMPDIR:-/tmp}/wordendcount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordendcount.png" \
    "echo warmup" "wordendcount . motd" "wordendcount e about.txt" \
    "wordendcount n about.txt" "wordendcount x nope" "wordendcount ." > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordendcount:|usage: wordendcount" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed contents from kernel/src/vfs.rs seed().
seed = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def wec(text, ch):
    return sum(1 for w in text.split() if w and w[-1] == ch)

cases = [("motd", "."), ("about.txt", "e"), ("about.txt", "n")]
for name, ch in cases:
    want = f"wordendcount: {wec(seed[name], ch)} words end with {ch}"
    if want not in s:
        print(f"WORDENDCOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly
assert "wordendcount: 1 words end with ." in s   # motd: OS.
assert "wordendcount: 6 words end with e" in s    # about.txt: one,codebase,...,device,software (6)
assert "wordendcount: 3 words end with n" in s    # about.txt: an, own, on

# missing file + no-arg
if "wordendcount: no such file: nope" not in s:
    print("WORDENDCOUNT TEST FAILED (missing-file path)"); sys.exit(1)
if "usage: wordendcount <ch> <file>" not in s:
    print("WORDENDCOUNT TEST FAILED (no-arg usage line)"); sys.exit(1)

print("WORDENDCOUNT TEST PASSED  (motd .->1, about.txt e/n->6/3 vs python; case-sensitive; "
      "missing-file + no-arg handled)")
PY
