#!/usr/bin/env bash
# 2.991 falsify-first probe: an SGI taken while the core is at EL0.
#
# Design: docs/design-2.991-sgi-el0-probe.md. Two arms on ONE boot, same SGI id
# (PROBE_SGI), same self-target, same core -- the ONLY difference is the
# exception entry point:
#   * SGI-EL1-SEEN  control, raised by the shell at EL1 -> exceptions.rs:55-59
#   * SGI-EL0-SEEN  subject, armed by an EL0 process    -> process.rs rust_irq_from_el0
#
# Verdicts:
#   EL1 1 / EL0 0 -> SGI-EL0-DROPPED     exit 10   (expected BEFORE the arm)
#   EL1 1 / EL0 1 -> SGI-EL0-DELIVERED   exit 0    (expected AFTER the arm)
#   EL1 0         -> PROBE-PREMISE-FAILED exit 20  (the probe falsified ITSELF:
#                    the SGI was never delivered anywhere, so the design's
#                    read of the GICD_SGIR encoding is what is wrong, not the
#                    kernel. This arm is why the control is on the same boot.)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "SGI-EL0-PROBE: build failed"; exit 30; }

SPORT="${SGI_PROBE_SERIAL_PORT:-4712}"
QPORT="${SGI_PROBE_QMP_PORT:-4713}"
OUT="${TMPDIR:-/tmp}/sgi-el0-probe-2991.out"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial "tcp:127.0.0.1:${SPORT},server,nowait" \
    -qmp "tcp:127.0.0.1:${QPORT},server,nowait" -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
python3 scripts/shell-demo.py "$SPORT" "$QPORT" "${TMPDIR:-/tmp}/sgi-el0-probe-2991.png" \
    "sgi-el0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

# ONE snapshot, then every assertion grades the SAME bytes (2.770).
SNAP="$(cat "$OUT")"
printf '%s\n' "$SNAP" | grep -aE "SGI-EL1-SEEN|SGI-EL0-SEEN|\[sgi-el0\]" | sed 's/^/  /'

pick() { printf '%s\n' "$SNAP" | grep -aoE "$1 [0-9]+" | tail -1 | awk '{print $2}'; }
EL1="$(pick 'SGI-EL1-SEEN')"
EL0="$(pick 'SGI-EL0-SEEN')"

if [ -z "$EL1" ] || [ -z "$EL0" ]; then
    echo "SGI-EL0-PROBE: MARKER MISSING (el1='${EL1:-none}' el0='${EL0:-none}') -- probe did not run"
    exit 21
fi
if [ "$EL1" -lt 1 ]; then
    echo "PROBE-PREMISE-FAILED  (SGI-EL1-SEEN $EL1: the SGI was not delivered at EL1 either;"
    echo "                       the self-target premise is what failed, not the EL0 entry point)"
    exit 20
fi
if [ "$EL0" -lt 1 ]; then
    echo "SGI-EL0-DROPPED  (SGI-EL1-SEEN $EL1, SGI-EL0-SEEN $EL0 -- same id, same core,"
    echo "                  same boot: rust_irq_from_el0 has no SGI arm and drops it)"
    exit 10
fi
echo "SGI-EL0-DELIVERED  (SGI-EL1-SEEN $EL1, SGI-EL0-SEEN $EL0)"
exit 0
