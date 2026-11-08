#!/usr/bin/env bash
# Verify the `capitalizelast <text>` command uppercases the LAST character of each
# whitespace-separated word (when a lowercase ASCII letter), leaving everything else
# unchanged: "hello world" -> "hellO worlD". The end-of-word mirror of title (first
# letter); distinct from capitalize. Self-checking: python replicates the per-word map.
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
OUT="${TMPDIR:-/tmp}/capitalizelast.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# "a bb" tests single-char + two-char words; "AfriSovera OS" tests already-upper last.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/capitalizelast.png" \
    "echo warmup" "capitalizelast hello world" "capitalizelast a bb" "capitalizelast AfriSovera OS" "capitalizelast solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "capitalizelast:|usage: capitalizelast" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def capitalizelast(t):
    out = []
    for w in t.split():
        b = list(w)
        if b and "a" <= b[-1] <= "z":
            b[-1] = b[-1].upper()
        out.append("".join(b))
    return " ".join(out)

for t in ["hello world", "a bb", "AfriSovera OS", "solo"]:
    want = f"capitalizelast: {capitalizelast(t)}"
    if want not in s:
        print(f"CAPITALIZELAST TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "capitalizelast: hellO worlD" in s
assert "capitalizelast: A bB" in s          # single char 'a'->A; 'bb'->bB
assert "capitalizelast: AfriSoverA OS" in s  # last 'a'->A; OS ends in upper S, unchanged
assert "capitalizelast: solO" in s          # single word

print("CAPITALIZELAST TEST PASSED  (hello world->hellO worlD, a bb->A bB, AfriSovera OS->AfriSoverA OS, "
      "solo->solO vs python; uppercase last letter of each word)")
PY
