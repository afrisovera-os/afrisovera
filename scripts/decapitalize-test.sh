#!/usr/bin/env bash
# Verify `decapitalize <text>` lowercases ONLY the first alphabetic character, leaving the rest
# unchanged -- the exact inverse of capitalize: "Hello WORLD"->"hello WORLD", "HELLO"->"hELLO",
# "hello"->"hello" (already lower), "123Abc"->"123abc", "AfriSovera OS"->"afriSovera OS".
# Self-checking: python lowercases the first alpha char only.
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
OUT="${TMPDIR:-/tmp}/decapitalize.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/decapitalize.png" \
    "echo warmup" "decapitalize Hello WORLD" "decapitalize HELLO" "decapitalize hello" \
    "decapitalize 123Abc" "decapitalize AfriSovera OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "decapitalize:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def decap(t):                         # DERIVED reference: lowercase the first alpha char only
    out = []
    done = False
    for ch in t:
        if not done and ch.isalpha():
            out.append(ch.lower()); done = True
        else:
            out.append(ch)
    return "decapitalize: " + "".join(out)

for t in ("Hello WORLD", "HELLO", "hello", "123Abc", "AfriSovera OS"):
    w = decap(t)
    if w not in s:
        print(f"DECAPITALIZE TEST FAILED (missing '{w}')"); sys.exit(1)

assert "decapitalize: hello WORLD" in s
assert "decapitalize: hELLO" in s
assert "decapitalize: hello" in s
assert "decapitalize: 123abc" in s
assert "decapitalize: afriSovera OS" in s

print("DECAPITALIZE TEST PASSED  (Hello WORLD->hello WORLD, HELLO->hELLO, hello->hello, "
      "123Abc->123abc, AfriSovera OS->afriSovera OS vs python first-alpha-lower)")
PY
