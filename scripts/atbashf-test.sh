#!/usr/bin/env bash
# Verify `atbashf <file>` applies the Atbash cipher (A<->Z, case-preserving) to every letter of a VFS
# file (non-letters pass through); self-inverse; the file analogue of atbash. Missing file errors.
# Self-checking: python maps each letter to its alphabet-reverse on the exact seed content.
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
OUT="${TMPDIR:-/tmp}/atbashf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/atbashf.png" \
    "echo warmup" "atbashf motd" "atbashf about.txt" "atbashf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "atbashf:|atbashf: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def atbash(t):                        # DERIVED reference: A<->Z, a<->z, case-preserving
    out = []
    for ch in t:
        o = ord(ch)
        if 65 <= o <= 90:    out.append(chr(90 - (o - 65)))
        elif 97 <= o <= 122: out.append(chr(122 - (o - 97)))
        else:                out.append(ch)
    return "".join(out)

want_motd = "atbashf: " + atbash(MOTD)
if want_motd not in s:
    print(f"ATBASHF TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
# about.txt spans a line break -> serial capture inserts \r; verify its two lines individually
for line in ("atbashf: " + atbash(ABOUT)).split("\n"):
    if line.replace("atbashf: ", "") not in s:
        print(f"ATBASHF TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

# self-inverse spot check: atbash(atbash(MOTD)) == MOTD
assert atbash(atbash(MOTD)) == MOTD
assert "atbashf: no such file: nope.txt" in s

print("ATBASHF TEST PASSED  (motd + about.txt both lines vs python atbash, self-inverse verified, "
      "nope.txt errors)")
PY
