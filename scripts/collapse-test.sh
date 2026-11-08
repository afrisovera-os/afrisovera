#!/usr/bin/env bash
# Verify the `collapse <text>` command collapses a run of ANY adjacent duplicate
# character down to a single one ("aaabbc"->"abc", "mississippi"->"misisipi"). It
# emits each byte only when it differs from the previously emitted byte. Distinct
# from squeeze (collapses runs of ONE GIVEN char, needs a char arg), uniqchars
# (removes ALL later dups across the whole string -- "mississippi"->"misp"), and
# sortchars (sorts). Self-checking: python keeps a byte only if it != previous.
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
OUT="${TMPDIR:-/tmp}/collapse.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "mississippi" contrasts collapse (misisipi) with uniqchars (misp).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/collapse.png" \
    "echo warmup" "collapse aaabbc" "collapse mississippi" "collapse aaa" "collapse" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "collapse:|usage: collapse" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def collapse(t):
    out = []; prev = None
    for c in t:
        if c != prev:
            out.append(c); prev = c
    return "".join(out)

for t in ["aaabbc", "mississippi", "aaa"]:
    want = f"collapse: {collapse(t)}"
    if want not in s:
        print(f"COLLAPSE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "collapse: abc" in s        # aaabbc -> abc
assert "collapse: misisipi" in s   # mississippi -> misisipi (adjacent only, NOT misp)
assert "collapse: a" in s          # aaa -> a
assert collapse("mississippi") == "misisipi" and "misisipi" != "misp"  # contrast with uniqchars

# empty input -> usage
if "usage: collapse <text>" not in s:
    print("COLLAPSE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("COLLAPSE TEST PASSED  (aaabbc->abc, mississippi->misisipi (adjacent, not "
      "uniqchars' misp), aaa->a vs python; empty -> usage; distinct from squeeze/uniqchars)")
PY
