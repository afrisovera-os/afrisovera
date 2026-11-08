#!/usr/bin/env bash
# Verify `sortwordchars <text>` sorts the bytes WITHIN each word ascending, keeping word order
# and single spaces: "hello world" -> "ehllo dlorw", "afri sovera os" -> "afir aeorsv os". The
# per-word canonical anagram form. Distinct from sortchars (whole text), sort (word order),
# reveach (reverses). Self-checking: python sorts each word's chars for the reference.
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
OUT="${TMPDIR:-/tmp}/sortwordchars.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sortwordchars.png" \
    "echo warmup" "sortwordchars hello world" "sortwordchars afri sovera os" \
    "sortwordchars dcba xyz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sortwordchars:|usage: sortwordchars" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def swc(text):  # mirror the kernel: sort each word's bytes, single-space join
    return " ".join("".join(sorted(w)) for w in text.split())

cases = ["hello world", "afri sovera os", "dcba xyz"]
for text in cases:
    want = f"sortwordchars: {swc(text)}"   # DERIVED by python, never hand-typed
    if want not in s:
        print(f"SORTWORDCHARS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "sortwordchars: ehllo dlorw" in s
assert "sortwordchars: afir aeorsv os" in s
assert "sortwordchars: abcd xyz" in s     # already-sorted word stays

print("SORTWORDCHARS TEST PASSED  (hello world->ehllo dlorw, afri sovera os->afir aeorsv os, "
      "dcba xyz->abcd xyz vs python per-word sorted)")
PY
