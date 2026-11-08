#!/usr/bin/env bash
# Verify `rot18f <file>` applies ROT18 (ROT13 letters + ROT5 digits, one pass) to a whole VFS
# file; non-letter/non-digit bytes (incl whitespace/line breaks) pass through. Self-inverse like
# rot18. The file analogue of rot18, completing the rot13f/atbashf/caesarf/rot47f/leetf cipher-
# file family. motd -> "Jrypbzr gb NsevFbiren BF -- n fbirervta, pncnovyvgl-frpherq BF." (same as
# rot13f since motd has no digits -- the letter half is exercised at file scope here; the digit
# half is already exercised by rot18/rot5). Missing file errors. Self-checking: python codecs
# rot13 + a manual digit-shift on the seed content.
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
OUT="${TMPDIR:-/tmp}/rot18f.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rot18f.png" \
    "echo warmup" "rot18f motd" "rot18f about.txt" "rot18f nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rot18f:|rot18f: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, codecs
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rot18(t):  # mirror the kernel: rot13 letters + rot5 digits, one pass
    out = []
    for c in t:
        if c.isdigit():
            out.append(chr((ord(c) - 48 + 5) % 10 + 48))
        elif c.isalpha():
            out.append(codecs.encode(c, "rot_13"))
        else:
            out.append(c)
    return "".join(out)

want_motd = "rot18f: " + rot18(MOTD)
if want_motd not in s:
    print(f"ROT18F TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
for line in ("rot18f: " + rot18(ABOUT)).split("\n"):
    if line.replace("rot18f: ", "") not in s:
        print(f"ROT18F TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "rot18f: Jrypbzr gb NsevFbiren BF -- n fbirervta, pncnovyvgl-frpherq BF." in s
assert "rot18f: no such file: nope.txt" in s

print("ROT18F TEST PASSED  (motd + about.txt both lines vs python rot13+rot5, nope.txt errors)")
PY
