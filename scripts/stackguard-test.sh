#!/usr/bin/env bash
# stackguard: the boot and secondary stacks each have an UNMAPPED guard below them, and running the boot stack
# out stops loudly instead of writing over .bss (F7, docs/design-f7-stack-guard.md).
#
# Boot 1 (two cores): `stack guards` asks the MMU (AT S1E1R) about the first and last byte of each 64 KiB guard
#   and the first byte of the stack above it: both guards must fault, both stacks must be mapped. `pmap` must
#   name both guards.
# Boot 2: `stack overflow-test` recurses on the boot stack with a 4 KiB written local per level. The run must
#   end in `[mansa] STACK OVERFLOW: boot stack ...` (printed lock-free: from the per-core emergency stack when
#   SP is inside the guard, or by the sync handler when only the faulting store is), must reach a depth that shows the
#   stack was really exhausted, and must never print the unreachable "returned" line.
# Without the guard the same dive runs on into .bss: no STACK OVERFLOW line, so this suite is red.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "STACKGUARD TEST FAILED (build)"; exit 1; }

T="${TMPDIR:-/tmp}"
OUT1="$T/stackguard-1.out"
OUT2="$T/stackguard-2.log"
. "$(dirname "$0")/lib/afrish-boot.sh"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 2 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4414,server,nowait \
    -qmp tcp:127.0.0.1:4415,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
python3 scripts/shell-demo.py 4414 4415 "$T/stackguard-1.png" "stack guards" "pmap" "stack" > "$OUT1" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

# Boot 2 on the serial console directly: the machine halts, so there is no prompt to come back to.
python3 - "$BIN" "$OUT2" <<'PY'
import subprocess, sys, time
binp, out = sys.argv[1], sys.argv[2]
p = subprocess.Popen(["qemu-system-aarch64", "-M", "virt", "-cpu", "cortex-a72", "-smp", "1", "-m", "256M",
                      "-nographic", "-kernel", binp],
                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
buf = b""
def read_until(marker, secs):
    global buf
    import os, select
    end = time.time() + secs
    fd = p.stdout.fileno()
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.5)
        if r:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            buf += chunk
            if marker in buf:
                return True
    return marker in buf
ok = read_until(b"AfriSh ready", 240)
if ok:
    time.sleep(0.5)
    p.stdin.write(b"stack overflow-test\r"); p.stdin.flush()
    read_until(b"[mansa] halting", 240)
    read_until(b"\x00never\x00", 3)  # a little more, to catch anything printed after the halt line
p.kill()
open(out, "wb").write(buf)
PY

grep -aE "\[stack\] guard|stack guard  |STACK OVERFLOW|overflow-test (diving|returned)|halting" "$OUT1" "$OUT2" | sed 's/^/  /'
grep -aE "overflow-test depth" "$OUT2" | tail -2 | sed 's/^/  /'

python3 - "$OUT1" "$OUT2" <<'PY'
import re, sys
a = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
b = open(sys.argv[2], "rb").read().decode("utf-8", "replace")
bad = []
for name in ("boot", "secondary"):
    if not re.search(r"\[stack\] guard %s 0x[0-9a-f]+\.\.0x[0-9a-f]+: first byte faults, last byte faults, "
                     r"stack above it mapped -> OK" % name, a):
        bad.append("boot 1: the %s guard is not an unmapped hole under a mapped stack" % name)
    if not re.search(r"stack guard\s+%s 0x[0-9a-f]+\.\.0x[0-9a-f]+ \(64 KiB" % name, a):
        bad.append("boot 1: pmap does not name the %s guard" % name)
if "[stack] guards OK" not in a:
    bad.append("boot 1: `stack guards` did not say OK")
if not re.search(r"\[stack\] boot 0x[0-9a-f]+\.\.0x[0-9a-f]+ size=1024 KiB", a):
    bad.append("boot 1: `stack` did not report the 1 MiB boot stack")
if re.search(r"SYNC EXCEPTION|STACK OVERFLOW|unhandled exception", a):
    bad.append("boot 1: a fault during a run that should have none")
if "AfriSh ready" not in b:
    bad.append("boot 2: the shell never came up")
# Two reporters, both lock-free: the vector's guard check (SP in the guard: "ran into its guard lo..hi",
# from the emergency stack) or rust_sync_handler (SP above it, the faulting store below: "the fault address
# is in its guard"). Either is the guard doing its job.
m = re.search(r"\[mansa\] STACK OVERFLOW: boot stack (ran into its guard 0x[0-9a-f]+\.\.0x[0-9a-f]+|\(the fault address is in its guard\))", b)
if not m:
    bad.append("boot 2: no STACK OVERFLOW report for the boot stack (an unguarded dive runs on into .bss)")
depths = [int(x) for x in re.findall(r"overflow-test depth (\d+)", b)]
if not depths or max(depths) < 64:
    bad.append("boot 2: the dive never got deep (max depth %s); the stack was not really exhausted" % (max(depths) if depths else "none"))
if "should be unreachable" in b:
    bad.append("boot 2: the dive RETURNED, so the stack never ran out")
if bad:
    print("STACKGUARD TEST FAILED")
    for x in bad:
        print("  - " + x)
    sys.exit(1)
print("STACKGUARD TEST PASSED  (both guards unmapped under mapped stacks; the boot-stack overrun stopped after "
      "%d dive levels: STACK OVERFLOW: boot stack %s)" % (max(depths), m.group(1)))
PY
