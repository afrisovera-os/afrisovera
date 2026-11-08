#!/usr/bin/env bash
# Verify `shoutf <file>` uppercases EVERY ASCII letter of a VFS file (other bytes pass through) --
# the file analogue of shout, forcing all-caps (unlike swapcasef which flips). motd -> "WELCOME TO
# AFRISOVERA OS -- A SOVEREIGN, CAPABILITY-SECURED OS.". Missing file errors. Self-checking: python
# uppercases the exact seed content.
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
OUT="${TMPDIR:-/tmp}/shoutf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/shoutf.png" \
    "echo warmup" "shoutf motd" "shoutf about.txt" "shoutf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "shoutf:|shoutf: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

# DERIVED reference: ASCII-uppercase every letter (python upper() suffices for this ASCII text)
want_motd = "shoutf: " + MOTD.upper()
if want_motd not in s:
    print(f"SHOUTF TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
# about.txt spans a line break -> serial capture inserts \r; verify its two lines individually
for line in ("shoutf: " + ABOUT.upper()).split("\n"):
    if line.replace("shoutf: ", "") not in s:
        print(f"SHOUTF TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "shoutf: WELCOME TO AFRISOVERA OS -- A SOVEREIGN, CAPABILITY-SECURED OS." in s
assert "AFRISOVERA: ONE CODEBASE, WASM-NATIVE APPS, NO GATEKEEPER STORE," in s
assert "AI AS AN OS SERVICE. YOU OWN THE DEVICE AND THE SOFTWARE ON IT." in s
assert "shoutf: no such file: nope.txt" in s

print("SHOUTF TEST PASSED  (motd -> ALL CAPS, about.txt both lines uppercased, nope.txt errors "
      "vs python upper())")
PY
