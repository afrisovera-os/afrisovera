#!/usr/bin/env bash
# Verify the `wordsperline <file>` command lists the number of whitespace-separated
# words in EACH line of a VFS file, space-separated (about.txt -> "8 14", motd ->
# "9"). The whole-file companion of linewords (one line by index). Self-checking:
# python computes per-line split().count() over the exact seed content.
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
OUT="${TMPDIR:-/tmp}/wordsperline.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# about.txt has TWO lines (real embedded \n) -> two counts; motd is one line; miss.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordsperline.png" \
    "echo warmup" "wordsperline about.txt" "wordsperline motd" "wordsperline nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordsperline:|usage: wordsperline" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed file contents (from kernel/src/vfs.rs seed()).
FILES = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def wordsperline(fname):
    return " ".join(str(len(line.split())) for line in FILES[fname].splitlines())

for fname in ["about.txt", "motd"]:
    want = f"wordsperline: {wordsperline(fname)}"   # counts DERIVED, never hand-typed
    if want not in s:
        print(f"WORDSPERLINE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: about.txt has two lines (8 then 14 words), motd one line (9 words)
assert "wordsperline: 8 14" in s
assert "wordsperline: 9" in s
assert "wordsperline: no such file: nope.txt" in s

print("WORDSPERLINE TEST PASSED  (about.txt->8 14, motd->9, missing-file handled vs python; "
      "per-line word count of a VFS file)")
PY
