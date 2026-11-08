#!/usr/bin/env bash
# Verify the `charsperline <file>` command lists the character (byte) length of EACH
# line of a VFS file, space-separated. The char analogue of wordsperline (words per
# line); distinct from linelen (one line by index). Self-checking: python computes
# per-line len() over the exact seed content (values DERIVED, never hand-typed).
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
OUT="${TMPDIR:-/tmp}/charsperline.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# about.txt has TWO lines -> two lengths; motd is one line; a missing file.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/charsperline.png" \
    "echo warmup" "charsperline about.txt" "charsperline motd" "charsperline nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "charsperline:|usage: charsperline" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed file contents (from kernel/src/vfs.rs seed()).
FILES = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def charsperline(fname):
    return " ".join(str(len(line)) for line in FILES[fname].splitlines())

for fname in ["about.txt", "motd"]:
    want = f"charsperline: {charsperline(fname)}"   # lengths DERIVED by python
    if want not in s:
        print(f"CHARSPERLINE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: motd is a single 62-char line; about.txt has two lines
mlen = len(FILES["motd"])
assert f"charsperline: {mlen}" in s
assert len(FILES["about.txt"].splitlines()) == 2
assert "charsperline: no such file: nope.txt" in s

print(f"CHARSPERLINE TEST PASSED  (motd->{mlen}, about.txt->{charsperline('about.txt')}, "
      "missing-file handled vs python; per-line char length of a VFS file)")
PY
