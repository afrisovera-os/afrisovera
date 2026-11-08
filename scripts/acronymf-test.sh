#!/usr/bin/env bash
# Verify `acronymf <file>` prints the acronym of a VFS file: the first char of each whitespace-
# delimited word, uppercased, joined across ALL lines. motd -> "WTAO-ASCO", about.txt ->
# "AOCWANGSAAAOSYOTDATSOI". The file analogue of acronym. Missing file errors. Self-checking:
# python derives the acronym from the exact seed content.
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
OUT="${TMPDIR:-/tmp}/acronymf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/acronymf.png" \
    "echo warmup" "acronymf motd" "acronymf about.txt" "acronymf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "acronymf:|acronymf: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def acronymf(content):                # DERIVED reference: uppercased first char of each word
    return "acronymf: " + "".join(w[0].upper() for w in content.split())

want_motd, want_about = acronymf(MOTD), acronymf(ABOUT)
for w in (want_motd, want_about):
    if w not in s:
        print(f"ACRONYMF TEST FAILED (missing '{w}')"); sys.exit(1)

assert "acronymf: WTAO-ASCO" in s
assert "acronymf: AOCWANGSAAAOSYOTDATSOI" in s
assert "acronymf: no such file: nope.txt" in s

print("ACRONYMF TEST PASSED  (motd -> WTAO-ASCO, about.txt -> AOCWANGSAAAOSYOTDATSOI, "
      "nope.txt errors vs python uppercased first-of-each-word)")
PY
