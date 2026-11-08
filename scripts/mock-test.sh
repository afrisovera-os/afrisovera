#!/usr/bin/env bash
# Verify the `mock <text>` command applies alternating "mocking" case: toggle the
# case of each ASCII letter starting lowercase (letter 0 lower, letter 1 upper,
# ...); non-letters pass through unchanged and do NOT consume a toggle. DISTINCT
# from swapcase (swaps existing case), title, capitalize. Self-checking: python
# replicates the same algorithm and asserts each output.
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
OUT="${TMPDIR:-/tmp}/mock.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/mock.png" \
    "mock hello world" "mock AfriSovera" "mock abc" "mock a" "mock" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mock:|usage: mock" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def mock(t):
    out, i = [], 0
    for ch in t:
        if 'a' <= ch.lower() <= 'z':
            out.append(ch.lower() if i % 2 == 0 else ch.upper())
            i += 1
        else:
            out.append(ch)     # non-letter: passthrough, no toggle
    return ''.join(out)

cases = ["hello world", "AfriSovera", "abc", "a"]
for text in cases:
    want = f"mock: {mock(text)}"
    if want not in s:
        print(f"MOCK TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: alternates by position (mock), distinct from swapcase (inverts case)
assert mock("hello world") == "hElLo WoRlD"
assert mock("AfriSovera") == "aFrIsOvErA"   # ignores source case, alternates by position

if "usage: mock <text>" not in s:
    print("MOCK TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("MOCK TEST PASSED  (alternating case exact vs python; non-letters passthrough, empty->usage)")
PY
