#!/usr/bin/env bash
# 3.701 MEASURE-FIRST probe: is uart-rx-stress-test.sh:177's per-trial
# `end = time.time() + 3.0` a fixed WINDOW (always spent) or a CEILING
# (early-break the instant the echo+prompt lands, same shape already ruled
# for pi/dhcp/config/smpinfo/uart-rx-stress's OWN connect-retry loop)?
#
# The census (scripts/fixed-window-census.py --check) currently prices this
# as the LAST remaining FIXED-WINDOW class (row 33, py-elapsed-cmp-adjacent
# `end = time.time() + 3.0`, NOTE: "has an early break and is probably a
# ceiling too, but that is UNMEASURED, so it stays COUNTED").
#
# This probe exercises the IDENTICAL shape (same seed 20749, same CMD, same
# chunking/delay choices, same per-trial `end = time.time()+3.0` loop with
# the same break condition) against a REAL boot on a separate port (4886),
# never touching the registered suite or its port (4884), and prints the
# per-trial ELAPSED time so the margin against the 3.0s ceiling can be read
# directly. It does NOT edit scripts/uart-rx-stress-test.sh and is not
# itself a registered suite.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

if ! python3 -c "import socket,sys
s = socket.socket()
try:
    s.bind(('127.0.0.1', 4886))
except OSError:
    sys.exit(1)" 2>/dev/null; then
    echo "PROBE FAILED (127.0.0.1:4886 already bound)"
    exit 1
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4886,server,nowait \
    -qmp tcp:127.0.0.1:4887,server,nowait -kernel "$BIN" &
QPID=$!

. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

python3 - 4886 300 <<'PY'
import random, socket, sys, time

port, trials = int(sys.argv[1]), int(sys.argv[2])
random.seed(20749)
CMD = "echo the quick brown fox jumps over the lazy dog"

BOOT_CEILING = 60.0
t_boot = time.time()
ser = None
while ser is None:
    try:
        ser = socket.create_connection(("127.0.0.1", port), timeout=10)
    except OSError as exc:
        if time.time() - t_boot >= BOOT_CEILING:
            print(f"PROBE FAILED (nothing listening on {port} after {BOOT_CEILING:.0f}s: {exc})")
            sys.exit(1)
        time.sleep(0.05)
print(f"  serial socket accepted after {time.time() - t_boot:.3f}s")
ser.settimeout(0.05)
log = bytearray()


def drain(cap):
    end = time.time() + cap
    while time.time() < end:
        try:
            b = ser.recv(4096)
            if b:
                log.extend(b)
        except socket.timeout:
            pass


drain(0.2)
end = time.time() + BOOT_CEILING
while time.time() < end and not bytes(log).rstrip().endswith(b"afrisovera>"):
    ser.sendall(b"\r")
    drain(1.0)
if not bytes(log).rstrip().endswith(b"afrisovera>"):
    print("PROBE FAILED (no prompt for a bare newline)")
    sys.exit(1)

elapsed_all = []
answered = 0
for trial in range(trials):
    start = len(log)
    data = (CMD + "\r").encode()
    step = random.choice([1, 2, 3, 4, 8, 16, len(data)])
    for i in range(0, len(data), step):
        ser.sendall(data[i:i + step])
        time.sleep(random.choice([0, 0.0005, 0.001, 0.002]))
    t0 = time.time()
    end = t0 + 3.0
    while time.time() < end:
        try:
            b = ser.recv(4096)
            if b:
                log.extend(b)
        except socket.timeout:
            pass
        if bytes(log[start:]).rstrip().endswith(b"afrisovera>"):
            break
    elapsed = time.time() - t0
    elapsed_all.append(elapsed)
    got = bytes(log[start:])
    if not got.rstrip().endswith(b"afrisovera>"):
        print(f"  command {trial + 1} of {trials} unanswered after {elapsed:.3f}s "
              f"(this trial rode the FULL 3.0s ceiling -- a real loss, not a window)")
        print(f"PROBE FAILED (console lost input after {answered} answered commands)")
        sys.exit(1)
    answered += 1

elapsed_all.sort()
n = len(elapsed_all)
print(f"PROBE PASSED  ({answered} of {trials} answered, none rode the 3.0s ceiling)")
print(f"  min={elapsed_all[0]:.4f}s  median={elapsed_all[n//2]:.4f}s  "
      f"p99={elapsed_all[int(n*0.99)]:.4f}s  max={elapsed_all[-1]:.4f}s  "
      f"ceiling=3.0000s  margin={3.0 - elapsed_all[-1]:.4f}s")
PY
RC=$?
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
exit $RC
