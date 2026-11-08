#!/usr/bin/env bash
# Verify the `digitsonly <text>` command keeps ONLY the ASCII digit bytes of the
# inline text, dropping every other byte ("ab12cd34"->"1234", "phone: 555-0199"
# ->"5550199"). A filter, distinct from disemvowel (drops vowels), despace
# (drops whitespace), squeeze (collapses runs), uniqchars (dedup). No digits ->
# "(none)". Self-checking: python's str.isdigit filter reproduces the output.
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
OUT="${TMPDIR:-/tmp}/digitsonly.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; keep the
# command COUNT modest (digitsonly output truncated at higher counts).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/digitsonly.png" \
    "echo warmup" "digitsonly ab12cd34" "digitsonly 2026-08-07" \
    "digitsonly nodigits" "digitsonly" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "digitsonly:|usage: digitsonly" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def digitsonly(t):
    d = "".join(c for c in t if c.isdigit())
    return d if d else "(none)"

cases = ["ab12cd34", "2026-08-07", "nodigits"]
for t in cases:
    want = f"digitsonly: {digitsonly(t)}"
    if want not in s:
        print(f"DIGITSONLY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms explicitly
assert "digitsonly: 1234" in s         # ab12cd34 -> keeps 1,2,3,4
assert "digitsonly: 20260807" in s      # 2026-08-07 -> dashes dropped
assert "digitsonly: (none)" in s        # nodigits -> no digit bytes

# empty input -> usage
if "usage: digitsonly <text>" not in s:
    print("DIGITSONLY TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("DIGITSONLY TEST PASSED  (ab12cd34->1234, 2026-08-07->20260807, nodigits->(none) "
      "vs python isdigit filter; empty -> usage)")
PY
