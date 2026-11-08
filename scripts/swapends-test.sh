#!/usr/bin/env bash
# Verify the `swapends <text>` command swaps the first and last character of the
# inline text, delimited by [ ] ("abcd" -> "dbca", "hello" -> "oellh"); a single
# character (or empty) is emitted unchanged. Distinct from swappairs (swaps every
# adjacent pair, "abcd"->"badc"), rev (reverses the whole string), and swapcase
# (inverts case). Self-checking: python t[-1] + t[1:-1] + t[0].
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
OUT="${TMPDIR:-/tmp}/swapends.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "ab" pins the 2-char case; "a" pins the single-char (unchanged) case.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swapends.png" \
    "echo warmup" "swapends abcd" "swapends AfriSovera" "swapends ab" "swapends a" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapends:|usage: swapends" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def se(t):
    return t if len(t) <= 1 else t[-1] + t[1:-1] + t[0]

for t in ["abcd", "AfriSovera", "ab", "a"]:
    want = f"swapends: [{se(t)}]"
    if want not in s:
        print(f"SWAPENDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "swapends: [dbca]" in s          # abcd -> dbca
assert "swapends: [afriSoverA]" in s    # AfriSovera -> case of first/last swaps position
assert "swapends: [ba]" in s            # ab -> ba (2-char)
assert "swapends: [a]" in s             # single char unchanged

print("SWAPENDS TEST PASSED  (abcd->dbca, AfriSovera->afriSoverA, ab->ba, a unchanged "
      "vs python; distinct from swappairs/rev/swapcase)")
PY
