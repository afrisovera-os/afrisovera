#!/usr/bin/env bash
# Verify `titlef <file>` title-cases every whitespace-delimited word of a VFS file (capitalize the
# first letter, lowercase the rest, pass through whitespace/punctuation/line breaks) -- the file
# analogue of `title`, mirroring swapcasef/revf. motd -> "Welcome To Afrisovera Os -- A Sovereign,
# Capability-secured Os.". Missing file errors. Self-checking: python replicates the byte-level rule.
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
OUT="${TMPDIR:-/tmp}/titlef.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/titlef.png" \
    "echo warmup" "titlef motd" "titlef about.txt" "titlef nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "titlef:|titlef: no such" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def titlef(content):                      # DERIVED reference: kernel's byte-level word title-case
    out = []
    start = True
    for ch in content:
        if ch.isspace():
            out.append(ch); start = True
        else:
            if start:
                out.append(ch.upper() if ch.islower() else ch)
            else:
                out.append(ch.lower() if ch.isupper() else ch)
            start = False
    return "titlef: " + "".join(out)

# Full single-line motd is checked whole; about.txt spans a line break, so the serial
# capture inserts \r between the lines -- verify its two lines individually below.
want_motd = titlef(MOTD)
if want_motd not in s:
    print(f"TITLEF TEST FAILED (missing '{want_motd!r}')"); sys.exit(1)
for line in titlef(ABOUT).split("\n"):
    if line.replace("titlef: ", "") not in s:
        print(f"TITLEF TEST FAILED (missing about.txt line '{line!r}')"); sys.exit(1)

assert "titlef: Welcome To Afrisovera Os -- A Sovereign, Capability-secured Os." in s
assert "titlef: Afrisovera: One Codebase, Wasm-native Apps, No Gatekeeper Store," in s
assert "Ai As An Os Service. You Own The Device And The Software On It." in s
assert "titlef: no such file: nope.txt" in s

print("TITLEF TEST PASSED  (motd -> Welcome To Afrisovera Os ..., about.txt both lines, "
      "nope.txt errors vs python byte-level word title-case)")
PY
