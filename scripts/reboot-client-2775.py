#!/usr/bin/env python3
"""Phase 2.775 — `reboot-test.sh`'s inline client, in BOTH shapes, one file.

The two shapes differ in exactly one thing: HOW THEY WAIT. Everything else —
the connect retry, the buffer, and above all the six assertions and their
verbatim messages — is shared source, because a comparison between two shapes
graded against two different bars means nothing. This is the same construction
as scripts/qemu-window-falsify-2771.sh's single `grade()`.

  MODE=old   drain(9) twice: a HARD 9 s per phase with NO fall-through. This is
             reboot-test.sh:37 and :41 as they stand at HEAD.
  MODE=new   wait on the GUEST'S OWN OUTPUT under a CEILING, exactly the
             contract afrish_wait_marker and shell-demo.py's wait_ready use:
             return the moment the guest has said the thing, fall through on
             the ceiling WITHOUT deciding anything, and let the suite's own
             assertion be what fails.

ENV
  MODE=old|new           (required)
  REBOOT_PORT=<port>     serial port to dial            (default 4662)
  REBOOT_WINDOW=<sec>    MODE=old: the drain window     (default 9)
  REBOOT_CEILING=<sec>   MODE=new: the per-phase ceiling(default 90)
"""
import os
import socket
import sys
import time

MODE = os.environ.get("MODE", "")
PORT = int(os.environ.get("REBOOT_PORT", "4662"))
WINDOW = float(os.environ.get("REBOOT_WINDOW", "9"))
CEILING = float(os.environ.get("REBOOT_CEILING", "90"))
if MODE not in ("old", "new"):
    print("reboot-client: MODE must be old|new", file=sys.stderr)
    sys.exit(2)

s = None
for _ in range(100):
    try:
        s = socket.create_connection(("127.0.0.1", PORT), timeout=1)
        break
    except OSError:
        time.sleep(0.1)
if s is None:
    print("REBOOT TEST FAILED (could not connect to serial)")
    sys.exit(1)
s.settimeout(1.0)
buf = bytearray()
t0 = time.time()


def drain(sec):
    """HEAD's wait: read for `sec` seconds and then stop, whatever happened."""
    end = time.time() + sec
    while time.time() < end:
        try:
            b = s.recv(4096)
            if b:
                buf.extend(b)
        except socket.timeout:
            pass


def wait_for(pred, ceiling, what):
    """The replacement: return on the guest's own output; the cap is a CEILING.

    Three properties this must have, and the third is the one that could make
    the fix worse than the defect:
      1. it returns the instant `pred` holds, so a healthy boot stops paying a
         slow host's margin (HEAD burns 9 s per phase unconditionally);
      2. on the ceiling it prints and RETURNS — it does not exit — so the
         verdict stays with the suite's own assertions;
      3. a CLOSED socket ends the wait. recv() on a dead peer returns b'' and
         never raises, so a `while time.time() < end` loop that only handles
         socket.timeout spins hot until the cap. At HEAD's 9 s that is a 9 s
         spin; at a 90 s ceiling it is a 90 s spin, i.e. the ceiling would have
         made a dead guest TEN TIMES more expensive. Arm 6 of the falsifier
         exists for this.
    """
    if pred():
        return True
    end = time.time() + ceiling
    while time.time() < end:
        try:
            b = s.recv(4096)
        except socket.timeout:
            continue
        except OSError:
            break
        if not b:
            print(f"reboot-test: serial closed while waiting for {what} after "
                  f"{ceiling - (end - time.time()):.1f}s — asserting anyway",
                  file=sys.stderr)
            return False
        buf.extend(b)
        if pred():
            return True
    print(f"reboot-test: {what} not seen within {ceiling:.0f}s "
          f"(harness ceiling, not a verdict) — asserting anyway", file=sys.stderr)
    return False


if MODE == "old":
    drain(WINDOW)                                    # boot 1
else:
    wait_for(lambda: b"AfriSh ready" in buf, CEILING, "boot-1 shell banner")

# ---- the assertions: shared by both shapes, verbatim from reboot-test.sh ----
if b"AfriSh ready" not in buf:
    print("REBOOT TEST FAILED (shell never came up in boot 1)")
    sys.exit(1)
s.sendall(b"reboot\r")

if MODE == "old":
    drain(WINDOW)                                    # reset + boot 2
else:
    wait_for(lambda: buf.count(b"AfriSh ready") >= 2, CEILING, "boot-2 shell banner")

txt = buf.decode("latin1")
n = txt.count("AfriSh ready")
if "[reboot]" not in txt:
    print("REBOOT TEST FAILED (reboot command not acknowledged)")
    sys.exit(1)
if n < 2:
    print(f"REBOOT TEST FAILED (AfriSh ready seen {n}x; PSCI reset did not restart the machine)")
    sys.exit(1)
print(f"REBOOT TEST PASSED  (PSCI SYSTEM_RESET restarted the CPU: shell banner seen {n}x)")
