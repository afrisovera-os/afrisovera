#!/usr/bin/env bash
# Prove persistent system config across reboots. The kernel auto-writes
# afrisovera.conf (hostname + DHCP lease + boot counter) to the virtio-blk disk
# at boot and restores it on the next boot. Two boots share one disk image; the
# boot counter must advance from 1 to 2, proving the config came from disk.
# Uses -serial stdio so the early boot-time '[cfg]' line is captured reliably.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
IMG="${TMPDIR:-/tmp}/afrisovera-config.img"
rm -f "$IMG"
head -c 4194304 /dev/zero > "$IMG" # fresh 4 MiB blank disk

# The strings this suite grades on, per boot, as ONE list each -- used BOTH to
# wait and to assert, so the wait cannot be satisfied by a different bar than
# the verdict (the `pi` shape, 2.783; the `dhcp` shape, 2.788). Boot 1 must
# INITIALIZE the config on a blank disk; boot 2 must LOAD it back and show the
# counter advanced. `ip=10.0.2.15` is deliberately NOT in either list: it is
# printed as a bonus and not graded (`|| true` below), and a wait list may only
# contain what the suite actually fails on.
CFG1_RE=(
    "cfg\] initialized; hostname=afrisovera boots=1"
)
CFG2_RE=(
    "cfg\] loaded from disk"
    "boots=2"
)

# This was `timeout 7 qemu ... ; grep`, TWICE -- kill the guest at a constant,
# then grep the corpse. As in `dhcp` (2.788) the constant was not the quantity
# it looked like: the '[cfg]' line lands ~1-2 s into boot, but the window was
# sized for the whole boot demo reaching the shell prompt, so both boots paid
# the full 7 s on every green run. A slower host would not have failed late; it
# would have printed `CONFIG TEST FAILED (boot 1 did not initialize the
# config)` -- a harness fault under the kernel's name.
#
# The 60 s below is a CEILING, not a window: the wait returns the moment every
# marker for that boot is present, and on expiry it returns 1 WITHOUT exiting,
# so the suite's own assertions name the failure. `timeout` stays only as a
# backstop in case the kill cannot reap the child; it is not what this suite
# grades against, and it is listed with that reason in fixed-window-census.py's
# EXCLUDED table so it is never silent.
. "$(dirname "$0")/lib/afrish-boot.sh"

boot() { # boot(logfile, marker...) -> one snapshot of that boot on stdout
    local out="$1"; shift
    : >"$out"
    timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -display none \
        -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
        -serial stdio -kernel "$BIN" >"$out" 2>&1 &
    local qpid=$!
    afrish_wait_markers "$out" 60 "$@" >&2
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
    # ONE snapshot, taken after the writer is reaped: every grep below grades the
    # SAME bytes, so the verdict is a property of the guest and not of when each
    # grep ran (2.770 measured a MISS following a HIT on a still-growing log).
    cat "$out"
}

L1="${TMPDIR:-/tmp}/afrisovera-config-boot1.log"
L2="${TMPDIR:-/tmp}/afrisovera-config-boot2.log"

echo "[boot 1] fresh disk — kernel initializes config + syncs..."
B1="$(boot "$L1" "${CFG1_RE[@]}")"
echo "$B1" | grep -E "\[cfg\]" | sed 's/^/  /'
echo "$B1" | grep -qE "${CFG1_RE[0]}" \
    || { echo "CONFIG TEST FAILED (boot 1 did not initialize the config)"; exit 1; }

echo "[boot 2] same disk — kernel restores config + advances the boot counter..."
B2="$(boot "$L2" "${CFG2_RE[@]}")"
echo "$B2" | grep -E "\[cfg\]" | sed 's/^/  /'
echo "$B2" | grep -qE "${CFG2_RE[0]}" \
    || { echo "CONFIG TEST FAILED (boot 2 did not load the config from disk)"; exit 1; }
echo "$B2" | grep -qE "${CFG2_RE[1]}" \
    || { echo "CONFIG TEST FAILED (boot counter did not persist+advance to 2)"; exit 1; }
# Bonus: the DHCP lease acquired at boot should be persisted too.
echo "$B2" | grep -q "ip=10.0.2.15" \
    && echo "  (bonus) DHCP lease 10.0.2.15 persisted across reboot" || true

echo "CONFIG TEST PASSED  (afrisovera.conf survived reboot; boot counter 1 -> 2)"
