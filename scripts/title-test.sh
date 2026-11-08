#!/usr/bin/env bash
# Verify the `title` command title-cases each word (first letter up, rest down).
# Deterministic:
#   title the quick brown fox -> "The Quick Brown Fox"
#   title aFRIsOVERA os       -> "Afrisovera Os"   (rest of each word lowercased)
#   title hello123 world      -> "Hello123 World"  (digits pass through)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4772,server,nowait \
    -qmp tcp:127.0.0.1:4773,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/title.out"
python3 scripts/shell-demo.py 4772 4773 "${TMPDIR:-/tmp}/title.png" "title the quick brown fox" "title aFRIsOVERA os" "title hello123 world" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "title: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["title: The Quick Brown Fox", "title: Afrisovera Os", "title: Hello123 World"]
missing = [w for w in want if w not in s]
if missing:
    print(f"TITLE TEST FAILED (missing: {missing})"); sys.exit(1)
print("TITLE TEST PASSED  (The Quick Brown Fox; Afrisovera Os; Hello123 World)")
PY
