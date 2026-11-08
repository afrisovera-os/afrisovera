#!/usr/bin/env bash
# Verify `capitalizef <file>` uppercases ONLY the first alphabetic character found in a whole VFS
# file (scanning from the start, skipping any leading non-letters until the first letter), leaving
# every other byte (remaining letters, digits, punctuation, newlines) completely unchanged -- the
# file analogue of `capitalize` (inline), mirroring titlef/swapcasef/shoutf/whisperf.
#
# motd and about.txt both already start with an uppercase letter, so capitalizef on them is a
# legitimate but WEAK identity-transform test (it proves pass-through, not the uppercase logic
# itself). To get a REAL assertion that exercises the transform, this test first `write`s two
# custom fixture files (following the pattern used by hexdump-test.sh/wc-test.sh/etc. that build
# their own fixtures via the shell's `write` command rather than relying only on seed files):
#   - fixture "cf1" = "hello world" (lowercase first letter)  -> "Hello world"
#   - fixture "cf2" = "123 lower"   (leading digits/space, first LETTER is lowercase 'l' in "lower")
#                                                              -> "123 Lower"
# Also covers: seed files (pass-through case) and a missing file (usage/error wording).
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
OUT="${TMPDIR:-/tmp}/capitalizef.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/capitalizef.png" \
    "echo warmup" "write cf1 hello world" "write cf2 123 lower" \
    "capitalizef cf1" "capitalizef cf2" \
    "capitalizef motd" "capitalizef about.txt" "capitalizef nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "capitalizef:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # serial renders embedded \n as \r\r\n -> strip all \r

# Known seed content (kernel/src/vfs.rs) -- both already start uppercase, so capitalizef is
# a byte-for-byte pass-through (weak but legitimate: proves the "already done" early-exit path).
MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT_LINE1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
ABOUT_LINE2 = "AI as an OS service. You own the device and the software on it."

checks = [
    # Real transform proof, via custom write-created fixtures (not identity):
    ("capitalizef: Hello world", "cf1 fixture 'hello world' -> 'Hello world'"),
    ("capitalizef: 123 Lower", "cf2 fixture '123 lower' -> '123 Lower' (skips leading digits/space)"),
    # Seed-file pass-through (already-uppercase first letter -> unchanged):
    (f"capitalizef: {MOTD}", "motd pass-through"),
    (f"capitalizef: {ABOUT_LINE1}", "about.txt line 1 pass-through"),
    (ABOUT_LINE2, "about.txt line 2 pass-through"),
    # Missing file:
    ("capitalizef: no such file: nope", "missing-file error wording"),
]

fails = [desc for want, desc in checks if want not in s]
if fails:
    print("CAPITALIZEF TEST FAILED (missing: " + "; ".join(fails) + ")")
    sys.exit(1)

print("CAPITALIZEF TEST PASSED  (cf1 'hello world'->'Hello world', "
      "cf2 '123 lower'->'123 Lower' [skips leading non-letters], "
      "motd/about.txt pass-through [already uppercase], nope->no such file; "
      "file analogue of capitalize)")
PY
