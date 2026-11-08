#!/usr/bin/env bash
# Verify the `firstn <n> <text>` command emits the first n bytes of a run of text,
# delimited by [ ]. If the text is shorter than n the whole text is emitted (n is
# clamped to the length). Distinct from chars (a character-class breakdown), cut
# (extracts a delimited field), wordn (extracts a word), and head (first n LINES
# of a VFS file) -- this is a raw char-prefix of inline text. Self-checking:
# python text[:n].
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
OUT="${TMPDIR:-/tmp}/firstn.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstn.png" \
    "echo warmup" "firstn 3 hello" "firstn 10 hi" "firstn 5 AfriSovera" "firstn" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstn:|usage: firstn" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (n, text) -> expected bracketed prefix text[:n] (clamped to the text length)
cases = [(3, "hello"), (10, "hi"), (5, "AfriSovera")]
for n, t in cases:
    want = f"firstn: [{t[:n]}]"
    if want not in s:
        print(f"FIRSTN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "firstn: [hel]" in s      # first 3 of "hello"
assert "firstn: [hi]" in s       # n=10 clamped to the 2-byte text
assert "firstn: [AfriS]" in s    # first 5 of "AfriSovera"

# empty input -> usage
if "usage: firstn <n> <text>" not in s:
    print("FIRSTN TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("FIRSTN TEST PASSED  (3/hello->hel, 10/hi clamped, 5/AfriSovera->AfriS vs "
      "python text[:n]; empty -> usage; distinct from chars/cut/wordn/head)")
PY
