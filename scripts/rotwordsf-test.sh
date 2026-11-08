#!/usr/bin/env bash
# Verify `rotwordsf <n> <file>` circular LEFT-rotates the WHITESPACE-separated WORD sequence of
# a whole VFS file by n (file analogue of rotwords/inline, completing the rotlines LINES /
# rotatef BYTES / rotwordsf WORDS file-rotation family). Words are read across ALL lines --
# embedded newlines are just whitespace -- and re-joined with single spaces.
# motd has 9 words: rotwordsf 1 motd drops the leading "Welcome" to the end. about.txt has
# 22 words (128 bytes incl embedded \n): rotwordsf 22 about.txt (n == count) is identity.
# Self-checking: python replicates words[k:] + words[:k] with k = n % len(words), space-joined.
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
OUT="${TMPDIR:-/tmp}/rotwordsf.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotwordsf.png" \
    "echo warmup" "rotwordsf 1 motd" "rotwordsf 22 about.txt" "rotwordsf 1 nope" "rotwordsf motd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotwordsf:|usage: rotwordsf" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
# the serial capture renders an embedded '\n' as '\r\r\n'; strip CRs so a multi-line
# body compares as a plain '\n'-joined string (the kernel output itself is correct).
s = raw.replace("\r", "")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rwf(text, n):  # mirror the kernel: circular left-rotate the WORD sequence by n mod count
    words = text.split()
    k = n % len(words)
    return " ".join(words[k:] + words[:k])

for text, n in [(MOTD, 1), (ABOUT, 22)]:
    want = f"rotwordsf: {rwf(text, n)}"
    if want not in s:
        print(f"ROTWORDSF TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the word-shift explicitly: rotwordsf 1 motd drops the leading word "Welcome" to the end
assert ("rotwordsf: to AfriSovera OS -- a sovereign, capability-secured OS. Welcome" in s), \
    "rotwordsf 1 motd should shift the leading word to the end"
# n == word count is identity (mod wrap); newlines collapse to single spaces
assert "rotwordsf: AfriSovera: one codebase, WASM-native apps, no gatekeeper store, AI as an OS service. You own the device and the software on it." in s, \
    "rotwordsf 22 (== count) should be identity (space-joined across the original line break)"

# error branches
assert "rotwordsf: nope not found" in s, "missing not-found branch"
assert "usage: rotwordsf <n> <file>" in s, "missing usage branch"

print("ROTWORDSF TEST PASSED  (motd 1->word-shifted, about.txt 22->identity (mod count), "
      "nope->not found, usage OK vs python)")
PY
