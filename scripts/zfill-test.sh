#!/usr/bin/env bash
# Verify the `zfill <width> <text>` command left-pads a run of text with ZEROS to
# a field of <width> columns, delimited by [ ] so the padding is unambiguous.
# Text already >= width is emitted unchanged. This is a SIGN-AGNOSTIC zero-fill:
# unlike Python str.zfill it does NOT move a leading sign in front of the zeros
# (zfill 5 -1 -> [000-1], not [-0001]). Distinct from rjust/indent/ljust (spaces).
# Self-checking: python compares against ('0'*(w-len)) + text.
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
OUT="${TMPDIR:-/tmp}/zfill.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "zfill 5 -1" pins the sign-agnostic behavior (zeros prepended, sign not moved).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/zfill.png" \
    "echo warmup" "zfill 6 42" "zfill 4 1234" "zfill 5 -1" "zfill" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "zfill:|usage: zfill" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# sign-agnostic zero-fill: ('0'*(w-len)) + text  (unchanged when len >= width)
def zf(w, t):
    n = len(t)
    return t if n >= w else ("0" * (w - n)) + t

cases = [(6, "42"), (4, "1234"), (5, "-1")]
for w, t in cases:
    want = f"zfill: [{zf(w, t)}]"
    if want not in s:
        print(f"ZFILL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "zfill: [000042]" in s   # 4 zeros + 42 = 6
assert "zfill: [1234]" in s     # text (4) >= width (4): unchanged
# sign-agnostic: zeros are prepended, the '-' is NOT moved to the front
assert "zfill: [000-1]" in s
assert "-1".zfill(5) == "-0001"   # (python str.zfill WOULD move the sign; we don't)

# empty input -> usage
if "usage: zfill <width> <text>" not in s:
    print("ZFILL TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("ZFILL TEST PASSED  (6/42->000042, 4/1234 unchanged, sign-agnostic 5/-1->000-1 "
      "not -0001; distinct from rjust/ljust/indent which pad spaces)")
PY
