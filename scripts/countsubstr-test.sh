#!/usr/bin/env bash
# Verify `countsubstr <sub> <text>` counts NON-OVERLAPPING occurrences of <sub> as a raw
# byte substring in the inline text, spanning word boundaries: "ss" in "mississippi" -> 2,
# "aa" in "aaaa" -> 2 (non-overlapping), "xyz" absent -> 0. The multi-char companion of
# countchar, distinct from countword (whole words). Self-checking: python's str.count is
# non-overlapping and computes every reference.
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
OUT="${TMPDIR:-/tmp}/countsubstr.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countsubstr.png" \
    "echo warmup" "countsubstr ss mississippi" "countsubstr an banana bandana" \
    "countsubstr aa aaaa" "countsubstr xyz hello world" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countsubstr:|usage: countsubstr" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# each case: (needle, text-after-first-space) exactly as the kernel parses `rest`
cases = [
    ("ss", "mississippi"),
    ("an", "banana bandana"),
    ("aa", "aaaa"),
    ("xyz", "hello world"),
]
for needle, text in cases:
    n = text.count(needle)   # python str.count is non-overlapping == the kernel scan
    want = f'countsubstr: "{needle}" occurs {n} times'
    if want not in s:
        print(f"COUNTSUBSTR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert 'countsubstr: "ss" occurs 2 times' in s
assert 'countsubstr: "aa" occurs 2 times' in s        # non-overlapping in "aaaa"
assert 'countsubstr: "xyz" occurs 0 times' in s       # absent

print("COUNTSUBSTR TEST PASSED  (ss/mississippi=2, aa/aaaa=2 non-overlap, xyz absent=0 "
      "vs python str.count)")
PY
