#!/usr/bin/env bash
# Verify `decapitalizef <file>` lowercases ONLY the first alphabetic character found in a whole VFS
# file (scanning from the start, skipping any leading non-letters until the first letter), leaving
# every other byte (remaining letters, digits, punctuation, newlines) completely unchanged -- the
# file analogue of `decapitalize` (inline), the exact inverse of `capitalizef`.
#
# motd and about.txt both start with an uppercase letter, so decapitalizef on them IS a real
# transform (unlike capitalizef's pass-through there). To also cover the "already lowercase"
# early-exit path, this test `write`s two custom fixture files (following the pattern used by
# capitalizef-test.sh/hexdump-test.sh/etc.):
#   - fixture "df1" = "Hello world" (uppercase first letter) -> "hello world"
#   - fixture "df2" = "123 Lower"   (leading digits/space, first LETTER is uppercase 'L') -> "123 lower"
#   - fixture "df3" = "already lower" (no-op identity case)  -> "already lower"
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
OUT="${TMPDIR:-/tmp}/decapitalizef.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/decapitalizef.png" \
    "echo warmup" "write df1 Hello world" "write df2 123 Lower" "write df3 already lower" \
    "decapitalizef df1" "decapitalizef df2" "decapitalizef df3" \
    "decapitalizef motd" "decapitalizef about.txt" "decapitalizef nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "decapitalizef:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # serial renders embedded \n as \r\r\n -> strip all \r

# Known seed content (kernel/src/vfs.rs) -- both start uppercase, so decapitalizef IS a real
# transform here (lowercases the leading letter, everything else untouched).
MOTD_IN = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
MOTD_OUT = "welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT_LINE1_IN = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
ABOUT_LINE1_OUT = "afriSovera: one codebase, WASM-native apps, no gatekeeper store,"
ABOUT_LINE2 = "AI as an OS service. You own the device and the software on it."

checks = [
    # Real transform proof, via custom write-created fixtures (not identity):
    ("decapitalizef: hello world", "df1 fixture 'Hello world' -> 'hello world'"),
    ("decapitalizef: 123 lower", "df2 fixture '123 Lower' -> '123 lower' (skips leading digits/space)"),
    ("decapitalizef: already lower", "df3 fixture already-lowercase pass-through"),
    # Seed-file real transform (leading uppercase letter -> lowered):
    (f"decapitalizef: {MOTD_OUT}", "motd first-letter lowered"),
    (f"decapitalizef: {ABOUT_LINE1_OUT}", "about.txt line 1 first-letter lowered"),
    (ABOUT_LINE2, "about.txt line 2 pass-through"),
    # Missing file:
    ("decapitalizef: no such file: nope", "missing-file error wording"),
]

fails = [desc for want, desc in checks if want not in s]
if fails:
    print("DECAPITALIZEF TEST FAILED (missing: " + "; ".join(fails) + ")")
    sys.exit(1)

print("DECAPITALIZEF TEST PASSED  (df1 'Hello world'->'hello world', "
      "df2 '123 Lower'->'123 lower' [skips leading non-letters], df3 already-lower pass-through, "
      "motd/about.txt first-letter lowered, nope->no such file; "
      "file analogue of decapitalize, inverse of capitalizef)")
PY
