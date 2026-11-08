#!/usr/bin/env bash
# Verify `reveachf <file>` reverses the CHARS of each word IN PLACE per line, keeping word ORDER and
# line structure -- the file analogue of reveach. motd -> "emocleW ot arevoSirfA SO -- a ,ngierevos
# deruces-ytilibapac .SO". Distinct from revf (reverses whole line) / rwordsf (word order) / tac.
# Missing file errors. Self-checking: python reverses each word per line.
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
OUT="${TMPDIR:-/tmp}/reveachf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/reveachf.png" \
    "echo warmup" "reveachf motd" "reveachf about.txt" "reveachf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "reveachf:|reveachf: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def reveachf(content):                # DERIVED reference: reverse each word's chars, per line
    return "\n".join(" ".join(w[::-1] for w in line.split()) for line in content.split("\n"))

want_motd = "reveachf: " + reveachf(MOTD)
if want_motd not in s:
    print(f"REVEACHF TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
# about.txt spans a line break -> serial capture inserts \r; verify its two lines individually
for line in ("reveachf: " + reveachf(ABOUT)).split("\n"):
    if line.replace("reveachf: ", "") not in s:
        print(f"REVEACHF TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "reveachf: emocleW ot arevoSirfA SO -- a ,ngierevos deruces-ytilibapac .SO" in s
assert ":arevoSirfA eno ,esabedoc evitan-MSAW ,sppa on repeeketag ,erots" in s
assert "reveachf: no such file: nope.txt" in s

print("REVEACHF TEST PASSED  (motd -> each word reversed, about.txt both lines, nope.txt errors "
      "vs python per-word reverse)")
PY
