#!/usr/bin/env bash
# Verify `countsentencesf <file>` counts sentences in a VFS FILE (file analogue of countsentences):
# motd -> 1 ("...OS."), about.txt -> 2 ("...service." + "...on it."), a miss -> "no such file".
# Same . ! ? terminator rule as countsentences. Self-checking against the known seed-file content.
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
OUT="${TMPDIR:-/tmp}/countsentencesf.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=4 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countsentencesf.png" \
    "echo warmup" "countsentencesf motd" "countsentencesf about.txt" \
    "countsentencesf nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countsentencesf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

# Known seed content (kernel/src/vfs.rs) -> DERIVED sentence counts via non-empty [.!?]-splits.
motd = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")

def nsent(t):
    return len([seg for seg in re.split(r"[.!?]", t) if seg.strip()])

exp_motd = f"countsentencesf: motd = {nsent(motd)} sentence(s)"     # 1
exp_about = f"countsentencesf: about.txt = {nsent(about)} sentence(s)"  # 2
for w in (exp_motd, exp_about):
    if w not in s:
        print(f"COUNTSENTENCESF TEST FAILED (missing '{w}')"); sys.exit(1)
if "countsentencesf: no such file: nope" not in s:
    print("COUNTSENTENCESF TEST FAILED (missing no-such-file branch)"); sys.exit(1)

print(f"COUNTSENTENCESF TEST PASSED  (motd->{nsent(motd)}, about.txt->{nsent(about)}, nope->no such "
      "file; VFS file analogue of countsentences)")
PY
