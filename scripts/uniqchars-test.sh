#!/usr/bin/env bash
# Verify the `uniqchars <text>` command removes DUPLICATE bytes from the inline
# text, keeping the FIRST occurrence of each in original order ("mississippi"->
# "misp", "hello"->"helo"). Distinct from squeeze (collapses only ADJACENT runs
# of one char), sortchars (sorts), dedup (list args), chars (per-char list).
# Self-checking: python's dict.fromkeys reproduces order-preserving dedup.
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
OUT="${TMPDIR:-/tmp}/uniqchars.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/uniqchars.png" \
    "echo warmup" "uniqchars mississippi" "uniqchars hello" \
    "uniqchars AfriSovera" "uniqchars abcabc" "uniqchars" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uniqchars:|usage: uniqchars" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

for t in ["mississippi", "hello", "AfriSovera", "abcabc"]:
    want = f"uniqchars: {''.join(dict.fromkeys(t))}"
    if want not in s:
        print(f"UNIQCHARS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms explicitly
assert "uniqchars: misp" in s       # mississippi
assert "uniqchars: helo" in s        # hello
assert "uniqchars: AfriSovea" in s   # AfriSovera: 2nd 'r' dropped, 'a' != 'A' (case-sensitive)
assert "uniqchars: abc" in s         # abcabc -> abc (non-adjacent dups removed, unlike squeeze)

# empty input -> usage
if "usage: uniqchars <text>" not in s:
    print("UNIQCHARS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("UNIQCHARS TEST PASSED  (mississippi->misp, hello->helo, AfriSovera->AfriSovea, "
      "abcabc->abc vs python dict.fromkeys; empty -> usage)")
PY
