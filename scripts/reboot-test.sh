#!/usr/bin/env bash
# Verify PSCI reboot: `reboot` issues PSCI SYSTEM_RESET (HVC conduit) which restarts
# the CPU, so the shell's "AfriSh ready" banner appears a SECOND time. The test
# connects to serial, captures boot 1, sends reboot, captures boot 2, and asserts
# the banner count is >= 2 (proving the reset actually re-ran the boot path).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4662,server,nowait \
    -qmp tcp:127.0.0.1:4663,server,nowait -kernel "$BIN" &
QPID=$!

python3 - <<'PY'
import os, socket, time, sys

# The wait is a CEILING, not a window (2.775). Until then this suite read for a
# hard 9s per boot phase with NO fall-through, and the 684-suite walk of 2.770
# red-ed it at row 42 with `shell never came up in boot 1` on a kernel that is
# independently green — a harness constant reported under the kernel's name
# (docs/audit-2.774-walk-reds-vs-census.md §3). Same name and default as the
# shared driver, which allows 90s for the same boot: one seam, one number.
#
# Worst case is 2 x CEILING = 180s against run-all-tests.sh's 300s per-suite
# timeout, so a genuinely dead guest still lands as this suite's own FAIL and
# not as a TIMEOUT that loses the assertion's name.
CEILING = float(os.environ.get("AFRISH_BOOT_CEILING", "90"))
s = None
for _ in range(100):
    try:
        s = socket.create_connection(("127.0.0.1", 4662), timeout=1); break
    except OSError:
        time.sleep(0.1)
if s is None:
    print("REBOOT TEST FAILED (could not connect to serial)"); sys.exit(1)
s.settimeout(1.0)
buf = bytearray()
def wait_for(pred, ceiling, what):
    """Read until the guest has said the thing; the cap is an upper bound only.

    Returns the moment `pred` holds (a healthy boot stops paying a slow host's
    margin), prints and RETURNS on the ceiling without deciding anything (the
    verdict stays with the assertions below), and ends on a CLOSED socket:
    recv() on a dead peer returns b'' and never raises, so a loop that handled
    only socket.timeout would spin hot for the whole 90s where the old 9s
    window gave up at 9 — the ceiling would have made a dead guest ten times
    more expensive. Falsified offline, six arms, zero boots:
    scripts/reboot-window-falsify-2775.sh -> FALSIFY-OK.
    """
    if pred(): return True
    end = time.time() + ceiling
    while time.time() < end:
        try:
            b = s.recv(4096)
        except socket.timeout:
            continue
        except OSError:
            break
        if not b:
            print("reboot-test: serial closed while waiting for %s after %.1fs"
                  " — asserting anyway" % (what, ceiling - (end - time.time())),
                  file=sys.stderr)
            return False
        buf.extend(b)
        if pred(): return True
    print("reboot-test: %s not seen within %.0fs (harness ceiling, not a"
          " verdict) — asserting anyway" % (what, ceiling), file=sys.stderr)
    return False
wait_for(lambda: b"AfriSh ready" in buf, CEILING, "boot-1 shell banner")
if b"AfriSh ready" not in buf:
    print("REBOOT TEST FAILED (shell never came up in boot 1)"); sys.exit(1)
s.sendall(b"reboot\r")
# The reset re-runs the boot path on the SAME serial socket, so boot 2 is
# "the banner a second time" — the suite's own verdict condition, waited on
# instead of slept through.
wait_for(lambda: buf.count(b"AfriSh ready") >= 2, CEILING, "boot-2 shell banner")
txt = buf.decode("latin1")
n = txt.count("AfriSh ready")
if "[reboot]" not in txt:
    print("REBOOT TEST FAILED (reboot command not acknowledged)"); sys.exit(1)
if n < 2:
    print(f"REBOOT TEST FAILED (AfriSh ready seen {n}x; PSCI reset did not restart the machine)"); sys.exit(1)
print(f"REBOOT TEST PASSED  (PSCI SYSTEM_RESET restarted the CPU: shell banner seen {n}x)")
PY
rc=$?
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
exit $rc
