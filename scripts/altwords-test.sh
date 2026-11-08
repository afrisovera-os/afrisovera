#!/usr/bin/env bash
# Verify the `altwords <text>` command alternates WORD case: uppercase every even-
# indexed word (0-based), lowercase every odd-indexed word: "hello world foo" ->
# "HELLO world FOO". The word-level analogue of mock (letter-level); distinct from
# title (each word title-cased). Self-checking: python replicates the per-word case.
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
OUT="${TMPDIR:-/tmp}/altwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# Mixed-case + digits + a single word exercise the toggle and passthrough.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/altwords.png" \
    "echo warmup" "altwords hello world foo bar" "altwords AfriSovera OS is Cool" "altwords hi" "altwords a1 b2 c3" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "altwords:|usage: altwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def altwords(t):
    return " ".join(w.upper() if i % 2 == 0 else w.lower()
                    for i, w in enumerate(t.split()))

for t in ["hello world foo bar", "AfriSovera OS is Cool", "hi", "a1 b2 c3"]:
    want = f"altwords: {altwords(t)}"
    if want not in s:
        print(f"ALTWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "altwords: HELLO world FOO bar" in s
assert "altwords: AFRISOVERA os IS cool" in s   # mixed case normalised per word
assert "altwords: HI" in s                       # single word -> uppercased
assert "altwords: A1 b2 C3" in s                 # digits pass through, case toggles

print("ALTWORDS TEST PASSED  (hello world foo bar->HELLO world FOO bar, "
      "AfriSovera OS is Cool->AFRISOVERA os IS cool, hi->HI, a1 b2 c3->A1 b2 C3 vs python)")
PY
