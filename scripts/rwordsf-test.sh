#!/usr/bin/env bash
# Verify `rwordsf <file>` reverses the WORD ORDER of each line of a VFS file (file analogue
# of rwords): motd->"OS. capability-secured sovereign, a -- OS AfriSovera to Welcome";
# about.txt's two lines each get their words reversed, newline-joined. Distinct from revf
# (chars) and tac (line order). Self-checking: python replicates " ".join(reversed(split)).
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
OUT="${TMPDIR:-/tmp}/rwordsf.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rwordsf.png" \
    "echo warmup" "rwordsf motd" "rwordsf about.txt" "rwordsf nope" "rwordsf" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rwordsf:|usage: rwordsf" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read().decode("latin1")
s = raw.replace("\r", "")  # capture renders embedded \n as \r\r\n

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
ABOUT = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def rw(text):  # mirror the kernel: reverse word order per line, newline-joined
    return "\n".join(" ".join(reversed(l.split())) for l in text.split("\n"))

for text in [MOTD, ABOUT]:
    want = f"rwordsf: {rw(text)}"
    if want not in s:
        print(f"RWORDSF TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "rwordsf: OS. capability-secured sovereign, a -- OS AfriSovera to Welcome" in s
# about.txt: line 2's reversed words follow line 1's reversed words, newline-joined
assert ("rwordsf: store, gatekeeper no apps, WASM-native codebase, one AfriSovera:\n"
        "it. on software the and device the own You service. OS an as AI") in s
# error branches
assert "rwordsf: no such file: nope" in s, "missing not-found branch"
assert "usage: rwordsf <file>" in s, "missing usage branch"

print("RWORDSF TEST PASSED  (motd + about.txt word orders reversed per line, "
      "nope->not found, usage OK vs python)")
PY
