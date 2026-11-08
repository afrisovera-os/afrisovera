#!/usr/bin/env bash
# Verify the `grepi <pattern> <file>` command does a CASE-INSENSITIVE line search
# over a VFS file, printing each line that contains <pattern> ignoring ASCII case.
# DISTINCT from grep (case-SENSITIVE), grepv (inverse), grepc (count), grepn (line
# numbers). The key proof is the control pair: `grepi afri about.txt` matches the
# "AfriSovera:" line while the case-sensitive `grep afri about.txt` matches nothing.
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
OUT="${TMPDIR:-/tmp}/grepi.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/grepi.png" \
    "grepi afri about.txt" "grep afri about.txt" "grepi WELCOME motd" \
    "grepi zzz about.txt" "grepi x nofile.txt" "grepi" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "AfriSovera:|Welcome to AfriSovera|no matches|no such file|usage: grepi" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

AFRI_LINE = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
MOTD_LINE = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

# 1) case-insensitive match: grepi afri (lowercase) finds the AfriSovera line.
if AFRI_LINE not in s:
    print("GREPI TEST FAILED (grepi afri did not match the AfriSovera line)"); sys.exit(1)

# 2) CONTROL: case-sensitive grep afri must NOT match -> exactly one "(no matches)"
#    for the grep control, plus one for the grepi zzz case = two total.
if s.count("(no matches)") < 2:
    print("GREPI TEST FAILED (expected grep afri AND grepi zzz to report no matches)"); sys.exit(1)

# 3) uppercase pattern WELCOME matches the (differently-cased) motd line.
if MOTD_LINE not in s:
    print("GREPI TEST FAILED (grepi WELCOME did not match the motd line)"); sys.exit(1)

# 4) missing file and usage messages.
if "grepi: no such file: nofile.txt" not in s:
    print("GREPI TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: grepi <pattern> <file>" not in s:
    print("GREPI TEST FAILED (missing usage line)"); sys.exit(1)

print("GREPI TEST PASSED  (case-insensitive match vs case-sensitive grep control; "
      "uppercase pattern matches; not-found + usage)")
PY
