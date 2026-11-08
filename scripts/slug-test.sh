#!/usr/bin/env bash
# Verify the `slug` command converts text to a URL-friendly slug: lowercase letters,
# keep digits, collapse every run of non-alphanumerics to a single '-', trim hyphens.
# Distinct from squeeze (collapses runs of ONE given char). The harness computes the
# slug the same way in python (lower, re.sub([^a-z0-9]+ -> '-'), strip('-')), so it is
# self-checking: "Hello, World!"->hello-world, "AfriSovera OS 2.0"->afrisovera-os-2-0,
# already-slugged unchanged, leading/trailing/embedded separators trimmed+collapsed.
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
OUT="${TMPDIR:-/tmp}/slug.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/slug.png" \
    "slug Hello, World!" "slug AfriSovera OS 2.0" "slug already-slugged" \
    "slug   trim  me  " "slug" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "slug:|usage: slug" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

def slug(t):
    return re.sub(r"[^a-z0-9]+", "-", t.lower()).strip("-")

phrases = ["Hello, World!", "AfriSovera OS 2.0", "already-slugged", "   trim  me  "]
for p in phrases:
    want = f"slug: {slug(p)}"
    if want not in s:
        print(f"SLUG TEST FAILED (missing '{want}' for {p!r})"); sys.exit(1)

if "usage: slug <text>" not in s:
    print("SLUG TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("SLUG TEST PASSED  (Hello World->hello-world, AfriSovera OS 2.0->afrisovera-os-2-0, trim+collapse; empty->usage)")
PY
