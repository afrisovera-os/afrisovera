#!/usr/bin/env bash
# Verify the `initials <text>` command emits the first character of each
# whitespace-separated word, joined with no separator and the ORIGINAL case
# preserved ("the quick brown fox" -> "tqbf", "AfriSovera OS project" -> "AOp").
# Distinct from acronym, which UPPERCASES the initials ("TQBF"). Self-checking:
# python "".join(w[0] for w in text.split()).
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
OUT="${TMPDIR:-/tmp}/initials.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "AfriSovera OS project" pins the case-preservation (AOp, not AOP).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/initials.png" \
    "echo warmup" "initials the quick brown fox" "initials AfriSovera OS project" "initials hello" "initials" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "initials:|usage: initials" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ini(t):
    return "".join(w[0] for w in t.split())

cases = ["the quick brown fox", "AfriSovera OS project", "hello"]
for t in cases:
    want = f"initials: {ini(t)}"
    if want not in s:
        print(f"INITIALS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "initials: tqbf" in s   # all lowercase preserved (acronym would give TQBF)
assert "initials: AOp" in s    # mixed case preserved (acronym would give AOP)
assert "initials: h" in s      # single word

# empty input -> usage
if "usage: initials <text>" not in s:
    print("INITIALS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("INITIALS TEST PASSED  (tqbf/AOp/h with original case preserved vs python; "
      "empty -> usage; distinct from acronym which uppercases)")
PY
