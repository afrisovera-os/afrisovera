#!/usr/bin/env bash
# Verify `rot13f <file>` applies ROT13 to every ASCII letter of a VFS file (non-letters pass through),
# the file analogue of rot13 (self-inverse). motd -> "Jrypbzr gb NsevFbiren BF -- n fbirervta,
# pncnovyvgl-frpherq BF.". Missing file errors. Self-checking: python codecs rot13 on the seed content.
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
OUT="${TMPDIR:-/tmp}/rot13f.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rot13f.png" \
    "echo warmup" "rot13f motd" "rot13f about.txt" "rot13f nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rot13f:|rot13f: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, codecs
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rot13(t):                         # DERIVED reference: standard ROT13 (letters only)
    return codecs.encode(t, "rot_13")

want_motd = "rot13f: " + rot13(MOTD)
if want_motd not in s:
    print(f"ROT13F TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
# about.txt spans a line break -> serial capture inserts \r; verify its two lines individually
for line in ("rot13f: " + rot13(ABOUT)).split("\n"):
    if line.replace("rot13f: ", "") not in s:
        print(f"ROT13F TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "rot13f: Jrypbzr gb NsevFbiren BF -- n fbirervta, pncnovyvgl-frpherq BF." in s
assert "rot13f: no such file: nope.txt" in s

print("ROT13F TEST PASSED  (motd + about.txt both lines vs python codecs rot_13, nope.txt errors)")
PY
