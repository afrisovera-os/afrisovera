#!/usr/bin/env bash
# Verify `mockf <file>` applies alternating mOcKiNg case over a whole VFS file: toggle each ASCII
# letter (letter 0 lower, 1 upper, ...); non-letters (incl whitespace/newlines) pass through and do
# NOT consume a toggle, so alternation continues across words and lines. File analogue of mock.
# Missing file errors. Self-checking: python replicates the letter-index toggle.
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
OUT="${TMPDIR:-/tmp}/mockf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mockf.png" \
    "echo warmup" "mockf motd" "mockf about.txt" "mockf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mockf:|mockf: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def mockf(content):                   # DERIVED reference: kernel's letter-index toggle
    out, i = [], 0
    for ch in content:
        if ch.isascii() and ch.isalpha():
            out.append(ch.lower() if i % 2 == 0 else ch.upper())
            i += 1
        else:
            out.append(ch)
    return "".join(out)

want_motd = "mockf: " + mockf(MOTD)
if want_motd not in s:
    print(f"MOCKF TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
# about.txt spans a line break -> serial capture inserts \r; verify its two lines individually
for line in ("mockf: " + mockf(ABOUT)).split("\n"):
    if line.replace("mockf: ", "") not in s:
        print(f"MOCKF TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "mockf: no such file: nope.txt" in s

print("MOCKF TEST PASSED  (motd + about.txt both lines vs python letter-index toggle, nope.txt errors)")
PY
