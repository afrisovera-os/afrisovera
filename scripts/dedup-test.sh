#!/usr/bin/env bash
# Verify the `dedup` command collapses ADJACENT duplicate words to one (like
# `uniq` on words); non-adjacent duplicates are preserved. Deterministic:
#   dedup the the cat cat cat sat -> "the cat sat"
#   dedup a b b a                 -> "a b a"   (trailing 'a' not adjacent to first)
#   dedup x y z                   -> "x y z"   (no dups)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4760,server,nowait \
    -qmp tcp:127.0.0.1:4761,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/dedup.out"
python3 scripts/shell-demo.py 4760 4761 "${TMPDIR:-/tmp}/dedup.png" "dedup the the cat cat cat sat" "dedup a b b a" "dedup x y z" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dedup: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["dedup: the cat sat", "dedup: a b a", "dedup: x y z"]
missing = [w for w in want if w not in s]
if missing:
    print(f"DEDUP TEST FAILED (missing: {missing})"); sys.exit(1)
print("DEDUP TEST PASSED  (the cat sat; a b a; x y z)")
PY
