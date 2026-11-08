#!/usr/bin/env bash
# Verify `everynth <n> <text>` keeps every nth character (stride-n decimation = text[::n]):
# "2 abcdef" -> ace, "3 abcdefghi" -> adg, "1 abc" -> abc, "2 hello" -> hlo, "4 abcdefgh" ->
# ae. Distinct from firstn/chunk/spaceout. Self-checking: python text[::n].
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
OUT="${TMPDIR:-/tmp}/everynth.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/everynth.png" \
    "echo warmup" "everynth 2 abcdef" "everynth 3 abcdefghi" "everynth 1 abc" \
    "everynth 2 hello" "everynth 4 abcdefgh" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "everynth:|usage: everynth" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):
    return "everynth: " + text[::n]   # DERIVED: Python stride slice

cases = [(2, "abcdef"), (3, "abcdefghi"), (1, "abc"), (2, "hello"), (4, "abcdefgh")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"EVERYNTH TEST FAILED (n={n} text={text} missing '{w}')"); sys.exit(1)

# pin the notable branches: stride 2, stride 3, identity stride 1, stride 2 odd len, stride 4
assert "everynth: ace" in s
assert "everynth: adg" in s
assert "everynth: abc" in s
assert "everynth: hlo" in s
assert "everynth: ae" in s

print("EVERYNTH TEST PASSED  (2 abcdef->ace, 3 abcdefghi->adg, 1 abc->abc, 2 hello->hlo, "
      "4 abcdefgh->ae vs python text[::n])")
PY
