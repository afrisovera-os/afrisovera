#!/usr/bin/env bash
# Verify the `sortchars <text>` command sorts the bytes of the inline text in
# ascending ASCII order (a permutation of the input characters). e.g.
# "dcba" -> "abcd", "banana" -> "aaabnn". DISTINCT from sort (sorts LINES of a
# VFS file / a list of NUMBERS), rev (reverses, no sort), squeeze/dedup (remove
# repeats). Self-checking: python's ''.join(sorted(t)) is the reference.
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
OUT="${TMPDIR:-/tmp}/sortchars.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sortchars.png" \
    "echo warmup" "sortchars dcba" "sortchars hello" "sortchars banana" \
    "sortchars a" "sortchars" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sortchars:|usage: sortchars" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sc(t):
    return ''.join(sorted(t))

cases = ["dcba", "hello", "banana", "a"]
for text in cases:
    want = f"sortchars: {sc(text)}"
    if want not in s:
        print(f"SORTCHARS TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: ascending sort, a permutation of the input
assert sc("dcba") == "abcd"
assert sc("hello") == "ehllo"
assert sc("banana") == "aaabnn"
assert sc("a") == "a"

if "usage: sortchars <text>" not in s:
    print("SORTCHARS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("SORTCHARS TEST PASSED  (dcba->abcd, hello->ehllo, banana->aaabnn exact vs "
      "python; empty->usage)")
PY
