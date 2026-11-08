#!/usr/bin/env bash
# Verify the `dotcase <text>` command: convert to dot.case -- like snake_case/kebab-case
# but joined by DOTS. Lowercases, splits camelCase humps, collapses runs of
# non-alphanumerics to a single '.', trims a trailing dot. Completes the separator trio
# snake='_', kebab='-', dotcase='.'. Distinct from slug (hyphens, no hump split) and
# camel/pascal (no separator). Self-checking: python replicates the transform and the
# separator-only contrast with snake.
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
OUT="${TMPDIR:-/tmp}/dotcase.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "fooBar baz" pins the camelCase hump split AND the space collapse in one case.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/dotcase.png" \
    "echo warmup" "dotcase hello world" "dotcase fooBar baz" "dotcase AfriSovera OS" "snake fooBar baz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dotcase:|snake:|usage: dotcase" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def conv(t, sep):
    # replicate the kernel: lowercase, split humps, collapse non-alnum to one sep, trim trailing
    b = t.encode("latin1")
    out = bytearray()
    for i, c in enumerate(b):
        ch = chr(c)
        if "A" <= ch <= "Z":
            if i > 0 and ("a" <= chr(b[i-1]) <= "z") and out and out[-1:] != sep:
                out += sep
            out.append(c | 0x20)
        elif ("a" <= ch <= "z") or ("0" <= ch <= "9"):
            out.append(c)
        else:
            if out and out[-1:] != sep:
                out += sep
    while out and out[-1:] == sep:
        out.pop()
    return out.decode("latin1")

for t in ["hello world", "fooBar baz", "AfriSovera OS"]:
    want = f"dotcase: {conv(t, b'.')}"
    if want not in s:
        print(f"DOTCASE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results + the separator-only contrast with snake
assert "dotcase: hello.world" in s
assert "dotcase: foo.bar.baz" in s            # camelCase hump split + space collapse
assert "dotcase: afri.sovera.os" in s
assert "snake: foo_bar_baz" in s              # same structure, '_' instead of '.'

print("DOTCASE TEST PASSED  (hello world->hello.world, fooBar baz->foo.bar.baz "
      "(hump split, vs snake foo_bar_baz), AfriSovera OS->afri.sovera.os vs python)")
PY
