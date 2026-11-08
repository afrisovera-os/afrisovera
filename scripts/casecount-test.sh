#!/usr/bin/env bash
# Verify `casecount <file>` counts the uppercase and lowercase ASCII letters in a VFS file
# (of the total letters): the case companion of vowelcount. motd -> 7 upper / 43 lower / 50
# letters; about.txt -> 11 upper / 89 lower / 100 letters; a missing file reports an error.
# Self-checking: python derives every count from the known seed content, never hand-typed.
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
OUT="${TMPDIR:-/tmp}/casecount.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/casecount.png" \
    "echo warmup" "casecount motd" "casecount about.txt" "casecount nope" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "casecount:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# the exact VFS seed content (kernel/src/vfs.rs seed())
SEED = {
    "motd": "Welcome to AfriSovera OS -- a sovereign, capability-secured OS.",
    "about.txt": ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
                  "AI as an OS service. You own the device and the software on it."),
}
for name, body in SEED.items():
    up = sum(1 for c in body if c.isascii() and c.isupper())
    lo = sum(1 for c in body if c.isascii() and c.islower())
    want = f"casecount: {name} = {up} uppercase, {lo} lowercase (of {up + lo} letters)"
    if want not in s:
        print(f"CASECOUNT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: the two seed tallies + the missing-file error
assert "casecount: motd = 7 uppercase, 43 lowercase (of 50 letters)" in s
assert "casecount: about.txt = 11 uppercase, 89 lowercase (of 100 letters)" in s
assert "casecount: no such file: nope" in s

print("CASECOUNT TEST PASSED  (motd=7/43/50, about.txt=11/89/100, missing-file errors "
      "vs python upper/lower tally)")
PY
