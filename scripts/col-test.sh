#!/usr/bin/env bash
# Verify `col <n> <file>` extracts the Nth (1-based) character (column) of each line of a VFS
# file, one per output line: the char analogue of cut (whitespace field) and the file/per-line
# analogue of charat. col 1 motd -> W; col 1 about.txt -> A, A; col 5 about.txt -> S, s; a line
# shorter than N -> (none). Self-checking: python derives each column char from the seed content.
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
OUT="${TMPDIR:-/tmp}/col.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/col.png" \
    "echo warmup" "col 1 motd" "col 1 about.txt" "col 5 about.txt" "col 99 motd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "col:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

SEED = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                  "AI as an OS service. You own the device and the software on it."),
}

def expected(name, n):  # mirror the kernel: nth char of each non-empty line, else (none)
    out = []
    for line in SEED[name].split("\n"):
        if not line:
            continue
        out.append(f"col: {line[n-1]}" if len(line) >= n else "col: (none)")
    return out

for name, n in [("motd", 1), ("about.txt", 1), ("about.txt", 5), ("motd", 99)]:
    for want in expected(name, n):   # every line DERIVED by python, never hand-typed
        if want not in s:
            print(f"COL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: first-char of each seed, an interior column, the short-line (none)
assert "col: W" in s                 # motd column 1
assert "col: A" in s                 # about.txt column 1 (both lines start A)
assert "col: S" in s and "col: s" in s   # about.txt column 5: line1 'S', line2 's'
assert "col: (none)" in s            # motd has < 99 chars

print("COL TEST PASSED  (col 1 motd->W, col 1 about.txt->A/A, col 5 about.txt->S/s, "
      "col 99 motd->(none) vs python per-line nth char)")
PY
