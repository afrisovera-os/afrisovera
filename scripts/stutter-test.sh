#!/usr/bin/env bash
# Verify the `stutter <text>` command prefixes each whitespace-separated word with
# its first character and a hyphen: "hello world" -> "h-hello w-world". Distinct from
# initials (first chars joined) and numeronym. Self-checking: python replicates
# " ".join(w[0] + "-" + w for w in t.split()).
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
OUT="${TMPDIR:-/tmp}/stutter.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# Multi-word + single word + single-char words exercise the spacing and edges.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stutter.png" \
    "echo warmup" "stutter hello world" "stutter own your device" "stutter hi" "stutter a b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stutter:|usage: stutter" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def stutter(t):
    return " ".join(w[0] + "-" + w for w in t.split())

for t in ["hello world", "own your device", "hi", "a b"]:
    want = f"stutter: {stutter(t)}"
    if want not in s:
        print(f"STUTTER TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "stutter: h-hello w-world" in s
assert "stutter: o-own y-your d-device" in s
assert "stutter: h-hi" in s          # single word
assert "stutter: a-a b-b" in s       # single-char words -> "x-x"

print("STUTTER TEST PASSED  (hello world->h-hello w-world, own your device->o-own y-your d-device, "
      "hi->h-hi, a b->a-a b-b vs python; first-letter-hyphen prefix per word)")
PY
