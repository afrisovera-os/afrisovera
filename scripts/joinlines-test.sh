#!/usr/bin/env bash
# Verify `joinlines <sep> <file>` joins a VFS file's lines with an arbitrary separator (the
# general form of unwrap): about.txt with "-" and "::" join its 2 lines; motd (1 line) prints
# unchanged; a missing file errors. Distinct from tac/join. Self-checking: python joins the
# known seed content's lines with the same separator.
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
OUT="${TMPDIR:-/tmp}/joinlines.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/joinlines.png" \
    "echo warmup" "joinlines - about.txt" "joinlines :: about.txt" \
    "joinlines @ motd" "joinlines - nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "joinlines:|usage: joinlines" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# exact seed content from vfs::seed()
MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def jl(sep, content):
    return "joinlines: " + sep.join(content.split("\n"))   # DERIVED via python line-join

for want in [jl("-", ABOUT), jl("::", ABOUT), jl("@", MOTD)]:
    if want not in s:
        print(f"JOINLINES TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: hyphen join, multi-char join, single-line unchanged, missing file
assert "joinlines: AfriSovera: one codebase, WASM-native apps, no gatekeeper store,-AI as an OS service. You own the device and the software on it." in s
assert "joinlines: AfriSovera: one codebase, WASM-native apps, no gatekeeper store,::AI as an OS service. You own the device and the software on it." in s
assert "joinlines: Welcome to AfriSovera OS -- a sovereign, capability-secured OS." in s
assert "joinlines: no such file: nope.txt" in s

print("JOINLINES TEST PASSED  (about.txt joined with - and ::, motd 1 line unchanged, "
      "nope.txt errors vs python sep.join(lines))")
PY
