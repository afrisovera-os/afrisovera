#!/usr/bin/env bash
# Verify the `lastn <n> <text>` command emits the last n bytes of a run of text,
# delimited by [ ]. If the text is shorter than n the whole text is emitted (n is
# clamped to the length). The mirror of firstn (which emits the FIRST n) -- same
# `<n> <text>` parse, but slices the tail bytes[len-take..]. Self-checking: python
# text[-n:].
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
OUT="${TMPDIR:-/tmp}/lastn.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastn.png" \
    "echo warmup" "lastn 3 hello" "lastn 10 hi" "lastn 4 AfriSovera" "lastn" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastn:|usage: lastn" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (n, text) -> expected bracketed suffix text[-n:] (clamped to the text length)
cases = [(3, "hello"), (10, "hi"), (4, "AfriSovera")]
for n, t in cases:
    want = f"lastn: [{t[-n:]}]"
    if want not in s:
        print(f"LASTN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "lastn: [llo]" in s       # last 3 of "hello"
assert "lastn: [hi]" in s        # n=10 clamped to the 2-byte text
assert "lastn: [vera]" in s      # last 4 of "AfriSovera"

# empty input -> usage
if "usage: lastn <n> <text>" not in s:
    print("LASTN TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("LASTN TEST PASSED  (3/hello->llo, 10/hi clamped, 4/AfriSovera->vera vs "
      "python text[-n:]; empty -> usage; mirror of firstn)")
PY
