#!/usr/bin/env bash
# Verify the `grepw <word> <file>` command does a WHOLE-WORD line search over a
# VFS file: it prints a line only where <word> appears as a complete word
# (bounded by non-alphanumeric chars or the line ends), not merely as a substring.
# DISTINCT from grep (any substring), grepi (case-insensitive), grepv (inverse),
# grepc (count), grepn (line numbers). The key proof is the control pair: in
# about.txt, `grepw on` matches ONLY line 2 ("...on it.") while the substring
# `grep on` ALSO matches line 1 (because "one" contains "on").
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
OUT="${TMPDIR:-/tmp}/grepw.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/grepw.png" \
    "echo warmup" "grepw on about.txt" "grep on about.txt" "grepw OS about.txt" \
    "grepw xyz about.txt" "grepw x nofile.txt" "grepw" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "codebase|on it|no matches|no such file|usage: grepw" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

LINE1 = "AfriSovera: one codebase, WASM-native apps, no gatekeeper store,"
LINE2 = "AI as an OS service. You own the device and the software on it."

# grepw on -> line 2 must appear (whole word "on" in "on it.").
if LINE2 not in s:
    print("GREPW TEST FAILED (grepw on did not match line 2)"); sys.exit(1)

# CONTROL: substring grep on ALSO matches line 1 ("one" contains "on"); grepw
# must NOT. So line 1 appears exactly ONCE in the whole capture (from grep on).
if s.count(LINE1) != 1:
    print(f"GREPW TEST FAILED (line 1 appeared {s.count(LINE1)}x; expected exactly 1 "
          "-- from the grep control only, NOT from grepw on)"); sys.exit(1)

# grepw OS matches line 2 (OS is a whole word) -> line 2 appears at least twice
# total (grepw on, grep on, grepw OS all print it): >= 3.
if s.count(LINE2) < 3:
    print(f"GREPW TEST FAILED (line 2 appeared {s.count(LINE2)}x; expected >=3)"); sys.exit(1)

if "(no matches)" not in s:
    print("GREPW TEST FAILED (grepw xyz should report no matches)"); sys.exit(1)
if "grepw: no such file: nofile.txt" not in s:
    print("GREPW TEST FAILED (missing not-found message)"); sys.exit(1)
if "usage: grepw <word> <file>" not in s:
    print("GREPW TEST FAILED (missing usage line)"); sys.exit(1)

print("GREPW TEST PASSED  (whole-word match vs substring grep control: grepw on -> "
      "line2 only, grep on -> also line1; grepw OS matches; not-found + usage)")
PY
