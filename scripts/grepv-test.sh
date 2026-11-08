#!/usr/bin/env bash
# Verify the `grepv` command prints the lines of a VFS file that do NOT contain a
# substring -- the inverse of grep. Uses the seeded two-line about.txt. Because
# grepv mirrors grep's bare-line output, the harness segments the capture per
# command (grepv AI -> line 1 only; grepv AfriSovera -> line 2 only; grepv zzz ->
# both). Also checks the no-such-file and usage lines.
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
OUT="${TMPDIR:-/tmp}/grepv.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/grepv.png" \
    "grepv AI about.txt" "grepv AfriSovera about.txt" "grepv zzz about.txt" \
    "grepv x nope.txt" "grepv" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "grepv|AfriSovera|AI as an OS|no such file|usage: grepv" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

LINE1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
LINE2 = "AI as an OS service. You own the device and the software on it."

# Commands were sent in this order; each command's echo delimits its output region.
C1 = "grepv AI about.txt"
C2 = "grepv AfriSovera about.txt"
C3 = "grepv zzz about.txt"
C4 = "grepv x nope.txt"

def region(start_cmd, end_cmd):
    a = s.index(start_cmd)
    a_end = a + len(start_cmd)
    b = s.index(end_cmd, a_end)
    return s[a_end:b]

try:
    r1 = region(C1, C2)   # grepv AI: line 1 kept, line 2 dropped (contains "AI")
    r2 = region(C2, C3)   # grepv AfriSovera: line 2 kept, line 1 dropped
    r3 = region(C3, C4)   # grepv zzz: both lines kept (neither contains "zzz")
except ValueError as e:
    print(f"GREPV TEST FAILED (could not segment capture: {e})"); sys.exit(1)

def need(cond, msg):
    if not cond:
        print(f"GREPV TEST FAILED ({msg})"); sys.exit(1)

need(LINE1 in r1 and LINE2 not in r1, "grepv AI should keep line 1 and drop line 2")
need(LINE2 in r2 and LINE1 not in r2, "grepv AfriSovera should keep line 2 and drop line 1")
need(LINE1 in r3 and LINE2 in r3, "grepv zzz should keep both lines")
need("grepv: no such file: nope.txt" in s, "missing no-such-file line")
need("usage: grepv <pattern> <file>" in s, "missing usage line")

print("GREPV TEST PASSED  (AI->line1 only, AfriSovera->line2 only, zzz->both; no-such-file + usage handled)")
PY
