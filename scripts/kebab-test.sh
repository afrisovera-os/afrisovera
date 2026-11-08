#!/usr/bin/env bash
# Verify the `kebab <text>` command converts text to kebab-case: lowercase, split
# BOTH on non-alphanumeric separators AND on camelCase humps, joined by single
# HYPHENS, acronym runs intact. DISTINCT from slug (hyphens but NO hump split),
# snake (UNDERSCORES + hump split), camel, pascal. Self-checking: python
# replicates the same byte-for-byte algorithm (identical to snake but '-').
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
OUT="${TMPDIR:-/tmp}/kebab.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/kebab.png" \
    "kebab helloWorld" "kebab own your device" "kebab AfriSovera OS project" \
    "kebab hello" "kebab" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "kebab:|usage: kebab" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def kebab(t):
    out = []
    for i, ch in enumerate(t):
        if 'A' <= ch <= 'Z':
            if i > 0 and 'a' <= t[i-1] <= 'z' and out and out[-1] != '-':
                out.append('-')
            out.append(ch.lower())
        elif ('a' <= ch <= 'z') or ('0' <= ch <= '9'):
            out.append(ch)
        else:
            if out and out[-1] != '-':
                out.append('-')
    while out and out[-1] == '-':
        out.pop()
    return ''.join(out)

cases = ["helloWorld", "own your device", "AfriSovera OS project", "hello"]
for text in cases:
    want = f"kebab: {kebab(text)}"
    if want not in s:
        print(f"KEBAB TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the distinctive transforms (hyphens + hump split, acronym intact)
assert kebab("helloWorld") == "hello-world"                   # camel hump split
assert kebab("AfriSovera OS project") == "afri-sovera-os-project"  # acronym OS intact
assert kebab("own your device") == "own-your-device"

if "usage: kebab <text>" not in s:
    print("KEBAB TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("KEBAB TEST PASSED  (kebab-case exact vs python; hyphen hump-split, acronym-intact, empty->usage)")
PY
