#!/usr/bin/env bash
# Verify the EL0 `SYS_SLEEP` syscall: the `syssleep` shell command spawns a real
# EL0 userspace app that asks the kernel to block it for ~300 ms via a syscall.
# The kernel waits on the always-running physical counter (works at EL1 with IRQs
# masked, unlike the tick-driven `sleep`) and reports the measured elapsed ms.
# Assert the process reported ~300 ms and exited cleanly. RAM-only.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4692,server,nowait \
    -qmp tcp:127.0.0.1:4693,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/sleep-syscall.out"
python3 scripts/shell-demo.py 4692 4693 "${TMPDIR:-/tmp}/sleep-syscall.png" "syssleep" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "syssleep|EL0|slept" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"slept (\d+)ms \(asked 300ms\)", s)
if not m:
    print("SLEEP-SYSCALL TEST FAILED (no EL0 sleep result line)"); sys.exit(1)
n = int(m.group(1))
if "@EL0] exit" not in s:
    print("SLEEP-SYSCALL TEST FAILED (EL0 app did not exit cleanly)"); sys.exit(1)
if 270 <= n <= 360:
    print(f"SLEEP-SYSCALL TEST PASSED  (EL0 process blocked {n} ms via SYS_SLEEP, ~300 ms as expected)")
    sys.exit(0)
print(f"SLEEP-SYSCALL TEST FAILED (expected ~300 ms, got {n})")
sys.exit(1)
PY
