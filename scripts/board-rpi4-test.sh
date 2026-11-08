#!/usr/bin/env bash
# Verify the Raspberry Pi 4 (BCM2711) peripheral map added in kernel/src/board.rs:
#   * the new `board-rpi4` feature builds (map-only board, no boot path yet);
#   * `board` still prints the ACTIVE map unchanged (qemu-virt);
#   * `board rpi4` / `board pi4` inspect the BCM2711 map from the virt build;
#   * `board list` lists all three known boards and marks the active one;
#   * `board <bogus>` reports the unknown name instead of silently doing nothing.
# Self-checking: python extracts the RPi4 UART + GIC bases from the blueprint table in
# docs/15-hardware-bringup.md and asserts the kernel prints exactly those addresses.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

# 1. The new feature must compile (built first; the default build below reclaims $BIN).
cargo build -p mansa-kernel --no-default-features --features board-rpi4 >/dev/null 2>&1 \
    || { echo "BOARD-RPI4 TEST FAILED (--features board-rpi4 does not build)"; exit 1; }
echo "  board-rpi4 feature build: OK"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/board-rpi4.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/board-rpi4.png" \
    "echo warmup" "board" "board rpi4" "board pi4" "board raspi" \
    "board list" "board dreamcast" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "board:|known boards|UART \(PL011\)|GIC |RTC |GPIO |fw_cfg|virtio-mmio base|map only|boots:|full-featured|^ +(virt|raspi|rpi4) +raspberry|^ +virt +qemu" "$OUT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md <<'PY'
import re, sys
s = open(sys.argv[1], "rb").read().decode("latin1")
doc = open(sys.argv[2], encoding="utf-8").read()

def norm(a):                              # 0xFE20_1000 -> 0xfe201000
    return a.replace("_", "").lower()

# DERIVED reference: pull the RPi4 (3rd data column) addresses out of the blueprint table.
def blueprint(row_label):
    m = re.search(r"^\|\s*" + row_label + r"\s*\|(.+)$", doc, re.M)
    assert m, f"blueprint row '{row_label}' not found in docs/15"
    col = m.group(1).split("|")[1]        # QEMU virt | >RPi4< | PINE64
    a = re.search(r"0x[0-9A-Fa-f_]+", col)
    assert a, f"no address in RPi4 column of '{row_label}': {col!r}"
    return norm(a.group(0))

uart = blueprint("Primary UART")          # 0xfe201000
gicd = blueprint("Interrupt ctrl")        # 0xff841000
assert (uart, gicd) == ("0xfe201000", "0xff841000"), (uart, gicd)
gicc = "0x%08x" % (int(gicd, 16) + 0x1000)  # GIC-400 CPU iface = distributor + 0x1000

def need(*frags):
    for f in frags:
        if f not in s:
            print(f"BOARD-RPI4 TEST FAILED (missing '{f}')"); sys.exit(1)

# `board` (no arg): the ACTIVE virt map, unchanged by this milestone.
need("board: qemu-virt (aarch64)",
     "UART (PL011)     @ 0x09000000",
     "GIC distributor  @ 0x08000000",
     "virtio-mmio base @ 0x0a000000",
     "(real boards define their own map; see docs/15)")

# `board rpi4`: the BCM2711 map, addresses matching the blueprint, absent
# peripherals printed as `none` (never as address 0).
need("board: raspberry-pi-4 (bcm2711)",
     "not active",                                  # inspection, not the booted board
     f"UART (PL011)     @ {uart}",
     f"GIC distributor  @ {gicd}",
     f"GIC cpu iface    @ {gicc}",
     "RTC  (PL031)     none",
     "GPIO             @ 0xfe200000",
     "fw_cfg           none",
     "virtio-mmio base none",
     "map only:")
if "@ 0x00000000" in s:
    print("BOARD-RPI4 TEST FAILED (a missing peripheral printed as address 0)"); sys.exit(1)

# The `pi4` alias must resolve to the same map (two BCM2711 blocks in the log).
if s.count("board: raspberry-pi-4 (bcm2711)") < 2:
    print("BOARD-RPI4 TEST FAILED (alias 'pi4' did not resolve to the BCM2711 map)"); sys.exit(1)

# `board raspi`: the Pi 3 map is honest too — PL011 rebased, no GIC at all.
need("board: raspberry-pi (bcm2837)", "UART (PL011)     @ 0x3f201000",
     "GIC distributor  none", "GPIO             @ 0x3f200000")

# `board list` + unknown-name handling.
need("known boards:", "virt   qemu-virt (aarch64) (active)",
     "raspi  raspberry-pi (bcm2837)", "rpi4   raspberry-pi-4 (bcm2711)",
     "board: unknown board 'dreamcast'")

print("BOARD-RPI4 TEST PASSED  (board-rpi4 builds; BCM2711 map "
      f"UART {uart} / GIC-400 {gicd}+{gicc} matches docs/15; list + alias + unknown OK)")
PY
