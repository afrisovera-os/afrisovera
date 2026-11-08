#!/usr/bin/env bash
# Verify the `snake <text>` command converts text to snake_case, completing the
# case-convention trio (slug=kebab/hyphens, camel=camelCase, snake=snake_case/
# underscores). It splits BOTH on non-alphanumeric separators AND on camelCase
# humps (uppercase preceded by a lowercase letter), while keeping acronym runs
# intact, and collapses/trims underscores. Self-checking: python replicates the
# same byte-for-byte algorithm and asserts the exact output per input.
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
OUT="${TMPDIR:-/tmp}/snake.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/snake.png" \
    "snake helloWorld" "snake own your device" "snake AfriSovera OS project" \
    "snake hello" "snake" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "snake:|usage: snake" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def snake(t):
    out = []
    for i, ch in enumerate(t):
        if 'A' <= ch <= 'Z':
            if i > 0 and 'a' <= t[i-1] <= 'z' and out and out[-1] != '_':
                out.append('_')
            out.append(ch.lower())
        elif ('a' <= ch <= 'z') or ('0' <= ch <= '9'):
            out.append(ch)
        else:
            if out and out[-1] != '_':
                out.append('_')
    while out and out[-1] == '_':
        out.pop()
    return ''.join(out)

cases = ["helloWorld", "own your device", "AfriSovera OS project", "hello"]
for text in cases:
    want = f"snake: {snake(text)}"
    if want not in s:
        print(f"SNAKE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the distinctive transforms
assert snake("helloWorld") == "hello_world"                 # camel hump split
assert snake("AfriSovera OS project") == "afri_sovera_os_project"  # acronym OS intact
assert snake("own your device") == "own_your_device"

if "usage: snake <text>" not in s:
    print("SNAKE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("SNAKE TEST PASSED  (snake_case exact vs python; hump-split, acronym-intact, spaces, empty->usage)")
PY
