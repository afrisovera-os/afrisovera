#!/usr/bin/env bash
# Verify the `wordcountf <word> <file>` command counts how many whole whitespace-
# delimited words in a VFS FILE exactly equal <word> (case-sensitive, punctuation
# kept). The file analogue of countword (inline), mirroring charcount vs countchar.
# Self-checking: python replicates split().count() over the exact seed content.
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
OUT="${TMPDIR:-/tmp}/wordcountf.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# "OS" in motd = 1 (the "OS." token is distinct); "the" in about.txt = 2; a miss = 0.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordcountf.png" \
    "echo warmup" "wordcountf OS motd" "wordcountf the about.txt" "wordcountf zzz motd" "wordcountf OS. motd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordcountf:|usage: wordcountf" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The exact seed file contents (from kernel/src/vfs.rs seed()).
FILES = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                 "AI as an OS service. You own the device and the software on it.",
}

def wcf(word, fname):
    return FILES[fname].split().count(word)   # count DERIVED, never hand-typed

cases = [("OS", "motd"), ("the", "about.txt"), ("zzz", "motd"), ("OS.", "motd")]
for word, fname in cases:
    n = wcf(word, fname)
    want = f"wordcountf: {word} occurs {n} times in {fname}"
    if want not in s:
        print(f"WORDCOUNTF TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: the notable cases resolve to the expected small integers
assert wcf("OS", "motd") == 1       # "OS." is a distinct token
assert wcf("the", "about.txt") == 2 # device's THE + software's THE
assert wcf("zzz", "motd") == 0

print("WORDCOUNTF TEST PASSED  (OS in motd=1, the in about.txt=2, zzz=0, OS.=1 vs python; "
      "whole-word occurrence count in a VFS file)")
PY
