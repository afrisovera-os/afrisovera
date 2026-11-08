#!/usr/bin/env bash
# Verify `sentencecasef <file>` sentence-cases a whole VFS file: uppercase ONLY the first
# alphabetic character found (scanning from the start, skipping any leading non-letters), and
# lowercase EVERY other letter in the file; non-letters (digits, punctuation, newlines) pass
# through unchanged -- the file analogue of `sentencecase` (inline), combining capitalizef's
# first-letter scan with decapitalizef-style whole-file rewriting.
#
# Both seed files start uppercase and contain mixed-case letters after the first, so
# sentencecasef on them IS a real transform (not pass-through). This test `write`s three custom
# fixture files (following the pattern used by capitalizef-test.sh/decapitalizef-test.sh):
#   - fixture "sf1" = "hello WORLD"   (lowercase first letter, mixed rest) -> "Hello world"
#   - fixture "sf2" = "123 hELLO"     (leading digits/space, first LETTER lowercase) -> "123 Hello"
#   - fixture "sf3" = "Already Sentencecase" (already correct)             -> "Already sentencecase"
# Also covers: seed files (real transform) and a missing file (usage/error wording).
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
OUT="${TMPDIR:-/tmp}/sentencecasef.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sentencecasef.png" \
    "echo warmup" "write sf1 hello WORLD" "write sf2 123 hELLO" "write sf3 Already Sentencecase" \
    "sentencecasef sf1" "sentencecasef sf2" "sentencecasef sf3" \
    "sentencecasef motd" "sentencecasef about.txt" "sentencecasef nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sentencecasef:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # serial renders embedded \n as \r\r\n -> strip all \r

# Known seed content (kernel/src/vfs.rs) -- both start uppercase but contain mixed-case letters
# after the first, so sentencecasef IS a real transform here (first letter forced upper, every
# other letter forced lower, non-letters untouched).
MOTD_OUT = "Welcome to afrisovera os -- a sovereign, capability-secured os."
ABOUT_LINE1_OUT = "Afrisovera: one codebase, wasm-native apps, no gatekeeper store,"
ABOUT_LINE2_OUT = "ai as an os service. you own the device and the software on it."

checks = [
    # Real transform proof, via custom write-created fixtures (not identity):
    ("sentencecasef: Hello world", "sf1 fixture 'hello WORLD' -> 'Hello world'"),
    ("sentencecasef: 123 Hello", "sf2 fixture '123 hELLO' -> '123 Hello' (skips leading digits/space)"),
    ("sentencecasef: Already sentencecase", "sf3 fixture 'Already Sentencecase' -> 'Already sentencecase'"),
    # Seed-file real transform (first letter forced upper, rest of letters forced lower):
    (f"sentencecasef: {MOTD_OUT}", "motd sentence-cased"),
    (f"sentencecasef: {ABOUT_LINE1_OUT}", "about.txt line 1 sentence-cased"),
    (ABOUT_LINE2_OUT, "about.txt line 2 sentence-cased"),
    # Missing file:
    ("sentencecasef: no such file: nope", "missing-file error wording"),
]

fails = [desc for want, desc in checks if want not in s]
if fails:
    print("SENTENCECASEF TEST FAILED (missing: " + "; ".join(fails) + ")")
    sys.exit(1)

print("SENTENCECASEF TEST PASSED  (sf1 'hello WORLD'->'Hello world', "
      "sf2 '123 hELLO'->'123 Hello' [skips leading non-letters], sf3 already-correct pass-through, "
      "motd/about.txt sentence-cased; file analogue of sentencecase, combines capitalizef's scan "
      "with decapitalizef-style whole-file rewriting)")
PY
