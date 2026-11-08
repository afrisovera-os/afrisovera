#!/usr/bin/env bash
# Verify the `smpinfo` command + the SMP secondary-core Aff0 recording.
#
# Two parts, because driving the interactive shell under -smp 2 is unreliable
# (core 1's async heartbeat lines interleave with the serial and break the
# prompt-sync that shell-demo relies on -- this is exactly why smp-test.sh only
# checks the boot log under -smp 2, never the shell):
#   Part A (-smp 1, interactive): the smpinfo command itself reports the live
#     single-core topology (primary Aff0 0; secondary not online).
#   Part B (-smp 2, boot log): the secondary comes up via PSCI and records its
#     OWN MPIDR Aff0 (= 1), which the primary echoes -- proving core 1 ran the
#     code that smpinfo reads.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

# --- Part A: interactive smpinfo under -smp 1 ---
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4716,server,nowait \
    -qmp tcp:127.0.0.1:4717,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
A="${TMPDIR:-/tmp}/smpinfo-a.out"
python3 scripts/shell-demo.py 4716 4717 "${TMPDIR:-/tmp}/smpinfo-a.png" "smpinfo" > "$A" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
echo "[A] -smp 1 interactive smpinfo:"; grep -aE "smpinfo: " "$A" | sed 's/^/    /'

# --- Part B: -smp 2 boot log (secondary Aff0 recorded by core 1) ---
# 2.869: the `sleep 12` that used to sit here was a FIXED WINDOW -- the suite
# killed the guest at a constant and greped the corpse, so it burned 12 s on
# every walk whether the secondary came up in 0.3 s or not at all. Replaced with
# the `pi`/`dhcp`/`config` shape (2.783/2.788): background the guest, wait for
# the MARKERS, kill, take ONE snapshot. SMP_RE is the assertion list from the
# grader below verbatim, so the wait cannot be satisfied by a different bar than
# the verdict. `timeout 60` is a BACKSTOP in case `kill` cannot reap the child,
# not the quantity graded; expiry of the WAIT returns 1 without exiting, so the
# grader's own `miss_b` is what names a failure.
B="${TMPDIR:-/tmp}/smpinfo-b.log"
SMP_RE=("SMP: 2 cores online" "secondary core Aff0 = 1")
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 2 -m 256M -nographic -kernel "$BIN" \
    >"$B" 2>&1 &
BPID=$!
afrish_wait_markers "$B" 45 "${SMP_RE[@]}"
kill "$BPID" 2>/dev/null
wait "$BPID" 2>/dev/null
echo "[B] -smp 2 boot topology:"; grep -aE "SMP: (2 cores|secondary core Aff0)" "$B" | sed 's/^/    /'

python3 - "$A" "$B" <<'PY'
import sys
a = open(sys.argv[1], "rb").read().decode("latin1")
b = open(sys.argv[2], "rb").read().decode("latin1")
want_a = ["cores online = 1", "primary core Aff0 = 0", "secondary core: not online"]
miss_a = [w for w in want_a if w not in a]
want_b = ["SMP: 2 cores online", "secondary core Aff0 = 1"]
miss_b = [w for w in want_b if w not in b]
if miss_a:
    print(f"SMPINFO TEST FAILED (Part A -smp 1 missing: {miss_a})"); sys.exit(1)
if miss_b:
    print(f"SMPINFO TEST FAILED (Part B -smp 2 boot log missing: {miss_b})"); sys.exit(1)
print("SMPINFO TEST PASSED  (-smp 1 command reports 1 core; -smp 2 secondary recorded Aff0 1)")
PY
