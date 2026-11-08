#!/usr/bin/env bash
# DHCP check: boot with a virtio-net device (modern transport) and confirm the
# kernel acquires an IP lease from QEMU's built-in SLIRP DHCP server at boot.
# QEMU user-net offers 10.0.2.15 / mask 255.255.255.0 / gw 10.0.2.2 / dns 10.0.2.3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
OUT="${TMPDIR:-/tmp}/dhcp-test.log"
: >"$OUT"

# The three strings this suite grades on, as ONE list, used BOTH to wait and to
# assert -- so the wait cannot be satisfied by a different bar than the verdict
# (the `pi` shape, 2.783). They are the whole DHCP handshake as this kernel
# prints it: DISCOVER out, OFFER in, and the ACK that installs the lease.
DHCP_RE=(
    "DHCP DISCOVER sent"
    "DHCP OFFER: 10\.0\.2\.15"
    "DHCP: IP 10\.0\.2\.15"
)

# This was `timeout 8 qemu ... ; grep` -- kill the guest at a constant, then grep
# the corpse. The constant was NOT the quantity it looked like: three measured
# runs of the old shape took 8405 / 8417 / 8597 ms wall (2.788a), i.e. every one
# of them rode the timeout to its boundary, so the suite was paying the full
# window on every green run and had ZERO measured margin -- a slower host does
# not fail late, it fails as `DHCP TEST FAILED (lease not acquired via ACK)`,
# a harness fault printed under the kernel's name.
#
# The 60 s below is a CEILING, not a window: the wait returns the moment all
# three strings are present (the lease lands ~1-2 s into boot, long before the
# shell prompt the old window was actually waiting for), and on expiry it
# returns 1 WITHOUT exiting, so the suite's own assertions name the failure.
# `timeout` stays only as a backstop in case the kill below cannot reap the
# child; it is not what this suite grades against, and it is listed with that
# reason in fixed-window-census.py's EXCLUDED table so it is never silent.
. "$(dirname "$0")/lib/afrish-boot.sh"
timeout 60 qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial stdio -kernel "$BIN" >"$OUT" 2>&1 &
QPID=$!
afrish_wait_markers "$OUT" 60 "${DHCP_RE[@]}"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

# ONE snapshot, three assertions, taken after the writer is reaped: every grep
# below grades the SAME bytes, so the verdict is a property of the guest and not
# of when each grep ran (2.770 measured a MISS following a HIT on a still-growing
# log in `smp`).
SNAP="$(cat "$OUT")"
have() { printf '%s\n' "$SNAP" | grep -qE "$1"; }

have "${DHCP_RE[0]}" || { echo "DHCP TEST FAILED (DISCOVER not sent)"; exit 1; }
have "${DHCP_RE[1]}" || { echo "DHCP TEST FAILED (no OFFER of 10.0.2.15)"; exit 1; }
have "${DHCP_RE[2]}" || { echo "DHCP TEST FAILED (lease not acquired via ACK)"; exit 1; }
echo "DHCP TEST PASSED  (acquired lease 10.0.2.15 from the SLIRP DHCP server)"
printf '%s\n' "$SNAP" | grep -E "DHCP" | sed 's/^/  /'
