#!/usr/bin/env bash
# Verify `constcase <text>` converts to CONSTANT_CASE (SCREAMING_SNAKE_CASE): same
# hump-splitting + separator collapsing as snake, but every letter UPPERCASED and
# words joined by '_'. "helloWorld"->HELLO_WORLD, "afri sovera os"->AFRI_SOVERA_OS.
# Distinct from snake (lowercase), kebab (hyphens), dotcase ('.'). Self-checking:
# python computes every reference with the same algorithm the kernel uses.
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
OUT="${TMPDIR:-/tmp}/constcase.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/constcase.png" \
    "echo warmup" "constcase helloWorld" "constcase afri sovera os" \
    "constcase my-var_name.42" "constcase AfriSoveraOS ID" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "constcase:|usage: constcase" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def constcase(text):  # mirror the kernel exactly
    buf = []
    b = text.encode("latin1")
    for i, c in enumerate(b):
        ch = chr(c)
        if 'A' <= ch <= 'Z':
            if i > 0 and 'a' <= chr(b[i-1]) <= 'z' and buf and buf[-1] != '_':
                buf.append('_')
            buf.append(ch)
        elif 'a' <= ch <= 'z':
            buf.append(ch.upper())
        elif '0' <= ch <= '9':
            buf.append(ch)
        else:
            if buf and buf[-1] != '_':
                buf.append('_')
    while buf and buf[-1] == '_':
        buf.pop()
    return ''.join(buf)

cases = {
    "helloWorld": "constcase helloWorld",
    "afri sovera os": "constcase afri sovera os",
    "my-var_name.42": "constcase my-var_name.42",
    "AfriSoveraOS ID": "constcase AfriSoveraOS ID",
}
for arg in cases:
    want = f"constcase: {constcase(arg)}"   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"CONSTCASE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly (camelCase hump split, space+hyphen+dot+digit)
assert "constcase: HELLO_WORLD" in s
assert "constcase: AFRI_SOVERA_OS" in s
assert "constcase: MY_VAR_NAME_42" in s
assert "constcase: AFRI_SOVERA_OS_ID" in s

print("CONSTCASE TEST PASSED  (helloWorld->HELLO_WORLD, afri sovera os->AFRI_SOVERA_OS, "
      "my-var_name.42->MY_VAR_NAME_42, AfriSoveraOS ID->AFRI_SOVERA_OS_ID vs python)")
PY
