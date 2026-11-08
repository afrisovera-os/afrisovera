#!/usr/bin/env bash
# Verify the `traincase <text>` command renders Train-Case (HTTP-Header-Case):
# capitalize each whitespace-separated word (first letter up, rest down) and join with
# hyphens -- "hello world"->"Hello-World", "content type"->"Content-Type". It is `title`
# joined by hyphens. Distinct from kebab (all-lowercase, hyphen), title (spaces), pascal
# (no separator), snake (underscores). Self-checking: python replicates the transform.
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
OUT="${TMPDIR:-/tmp}/traincase.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "content type" pins the classic HTTP-header shape; "hello WORLD foo" mixed-case input.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/traincase.png" \
    "echo warmup" "traincase hello world" "traincase content type" "traincase hello WORLD foo" "traincase solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "traincase:|usage: traincase" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def traincase(t):
    return "-".join(w[:1].upper() + w[1:].lower() for w in t.split())

for t in ["hello world", "content type", "hello WORLD foo", "solo"]:
    want = f"traincase: {traincase(t)}"
    if want not in s:
        print(f"TRAINCASE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "traincase: Hello-World" in s
assert "traincase: Content-Type" in s          # HTTP-Header-Case
assert "traincase: Hello-World-Foo" in s        # mixed-case input normalized per word
assert "traincase: Solo" in s                   # single word -> no hyphen

print("TRAINCASE TEST PASSED  (hello world->Hello-World, content type->Content-Type, "
      "hello WORLD foo->Hello-World-Foo, solo->Solo vs python; title joined by hyphens)")
PY
