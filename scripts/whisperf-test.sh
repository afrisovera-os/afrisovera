#!/usr/bin/env bash
# Verify `whisperf <file>` lowercases EVERY ASCII letter of a VFS file (other bytes pass through) --
# the file analogue of whisper and the lowercase counterpart of shoutf (forces all-lower, unlike
# swapcasef which flips). motd -> "welcome to afrisovera os -- a sovereign, capability-secured os.".
# Missing file errors. Self-checking: python lowercases the exact seed content.
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
OUT="${TMPDIR:-/tmp}/whisperf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/whisperf.png" \
    "echo warmup" "whisperf motd" "whisperf about.txt" "whisperf nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "whisperf:|whisperf: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

# DERIVED reference: ASCII-lowercase every letter (python lower() suffices for this ASCII text)
want_motd = "whisperf: " + MOTD.lower()
if want_motd not in s:
    print(f"WHISPERF TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
# about.txt spans a line break -> serial capture inserts \r; verify its two lines individually
for line in ("whisperf: " + ABOUT.lower()).split("\n"):
    if line.replace("whisperf: ", "") not in s:
        print(f"WHISPERF TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "whisperf: welcome to afrisovera os -- a sovereign, capability-secured os." in s
assert "afrisovera: one codebase, wasm-native apps, no gatekeeper store," in s
assert "ai as an os service. you own the device and the software on it." in s
assert "whisperf: no such file: nope.txt" in s

print("WHISPERF TEST PASSED  (motd -> all lowercase, about.txt both lines lowercased, nope.txt "
      "errors vs python lower())")
PY
