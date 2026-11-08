#!/usr/bin/env bash
# Verify the console idle-sleeps: the shell's input loop waits on `wfi` between
# keystrokes instead of busy-spinning. After a stretch of input silence the
# irqstat line reports a non-zero idle-wfi count (the shell actually slept and
# woke on the 100Hz timer / UART RX IRQ), while echo still returns cleanly.
#
# 2.776 — THE SILENCE IS CREATED AFTER THE PROMPT EXISTS, AND ITS PRECONDITION
# IS GRADED. Until now line 16 was `sleep 6   # let the shell idle-loop accrue
# wfi's before we type`: a HOST wait standing in for GUEST progress. The
# 684-suite walk of 2.770 red-ed this suite at row 34 with `idle-wfi is 0;
# shell not sleeping` on a kernel that is independently green — the capture
# says `irqs: timer 12 ticks` where the green run says 749, so the guest had
# executed ~120 ms of its own time inside a 6 s host sleep. The counter has one
# writer (`uart::note_idle_wfi`, kernel/src/uart.rs:246) and one call site
# (kernel/src/shell.rs:69, inside `cpu_getc` when the RX ring is empty), so
# `idle-wfi 0` at 12 ticks does not mean the shell refused to sleep — it means
# the guest never ran `cpu_getc` with an empty ring at all. The assertion was
# measuring HOST SCHEDULING and reporting it under the kernel's name, which is
# the signature of the fixed-window class (docs/audit-2.770-fixed-window-suites.md
# §6, docs/audit-2.774-walk-reds-vs-census.md §3).
#
# Two changes, and the second is the one that stops this recurring:
#   1. The idle window is shell-demo.py's bare ":" payload — a raw send of
#      NOTHING followed by 0.9 s of no input (scripts/shell-demo.py:186-190).
#      It is entered only after wait_ready() has seen the prompt, which is the
#      property the host `sleep 6` never had. No constant lives in this file
#      any more: the remaining number is the shared driver's, already ruled and
#      counted once (scripts/fixed-window-census.py KNOWN_DRIVERS).
#   2. The suite ASSERTS ITS OWN PRECONDITION. `uptime` before the silence and
#      irqstat's `timer N ticks` after it bracket the window in GUEST time. If
#      the guest advanced, `idle-wfi > 0` is a consequence of a verified fact;
#      if it did not, this suite says THE GUEST GOT NO CPU — which is true —
#      instead of "the shell is not sleeping", which is the false accusation it
#      made at row 34.
# Falsified offline, five arms, zero boots: scripts/shell-wfi-falsify-2776.sh.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4642,server,nowait \
    -qmp tcp:127.0.0.1:4643,server,nowait -kernel "$BIN" &
QPID=$!
OUT="${TMPDIR:-/tmp}/shell-wfi.out"
# "uptime" first (the tick reading that opens the window), then ":" — a raw
# payload of zero bytes, i.e. input silence with the shell already at a prompt.
python3 scripts/shell-demo.py 4642 4643 "${TMPDIR:-/tmp}/shell-wfi.png" \
    "uptime" ":" "echo HELLO" "irqstat" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "^HELLO|idle-wfi|^[0-9]+ ticks" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import os, sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

# Guest ticks the window must advance by for the idle-wfi reading to mean
# anything. 10 ticks is 100 ms of guest time inside a >=0.9 s host window —
# ~11% of one CPU. Below that the guest is starved, and this suite has nothing
# to say about the console's sleep behaviour. It is a floor on the PRECONDITION,
# not a window the verdict is graded against: a healthy box measures ~90.
MIN_TICKS = int(os.environ.get("AFRISH_WFI_MIN_TICKS", "10"))

if not re.search(r"(^|\n)HELLO\b", s):
    print("SHELL-WFI TEST FAILED (echo did not return a clean HELLO)"); sys.exit(1)
mu = re.search(r"(?m)^(\d+) ticks\s+\(~", s)
if not mu:
    print("SHELL-WFI TEST FAILED (uptime did not report a tick count)"); sys.exit(1)
mi = re.search(r"timer (\d+) ticks", s)
m = re.search(r"idle-wfi (\d+)", s)
if not (mi and m):
    print("SHELL-WFI TEST FAILED (irqstat did not report idle-wfi count)"); sys.exit(1)
t0, t1, n = int(mu.group(1)), int(mi.group(1)), int(m.group(1))
d = t1 - t0

# THE ORDER IS THE FIX. The precondition is consulted ONLY on the branch where
# the assertion has already failed — it decides WHICH failure this is, it does
# not get a veto over a passing one. Graded first (the shape this file carried
# for one hour of 2.776), it is a second window with a second bar, and a guest
# that idled only a little but DID idle — idle-wfi > 0 under MIN_TICKS — turns
# from HEAD's green into a new red. A repair for a class whose whole complaint
# is harness constants manufacturing reds must not add one. Measured, both
# directions, arms F and G of scripts/shell-wfi-falsify-2776.sh.
if n > 0:
    print(f"SHELL-WFI TEST PASSED  (clean echo HELLO; console idled on wfi {n} "
          f"times -> sleeps between keys; guest advanced {d} ticks across the "
          f"input silence)")
    sys.exit(0)
if d < MIN_TICKS:
    print(f"SHELL-WFI TEST FAILED (the guest got no CPU across the idle window: "
          f"uptime {t0} ticks -> irqstat {t1} ticks = {d} ticks of guest time, "
          f"under the {MIN_TICKS} required; host starvation, NOT a kernel verdict)")
    sys.exit(1)
print(f"SHELL-WFI TEST FAILED (idle-wfi is {n} after {d} ticks of verified "
      f"guest idle; shell not sleeping)")
sys.exit(1)
PY
