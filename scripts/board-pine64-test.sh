#!/usr/bin/env bash
# Verify the PINE64 / Allwinner A64 peripheral map added in kernel/src/board.rs:
#   * the new `board-pine64` feature builds (map-only board, no boot path yet);
#   * `board pine64` / `board a64` inspect the A64 map from the virt build;
#   * the map names the UART *part* — 8250dw, NOT PL011 — and prints the caveat that
#     this board needs its own UART driver, so "map complete" can never read as
#     "board ported";
#   * `board list` now lists all four known boards;
#   * the label refactor did not change what `board` prints for the PL011 boards.
# Self-checking: python extracts the PINE64 UART + GIC bases from the blueprint table
# in docs/15-hardware-bringup.md and asserts the kernel prints exactly those, and that
# the blueprint still calls that UART a DesignWare part.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

# 1. The new feature must compile (built first; the default build below reclaims $BIN).
cargo build -p mansa-kernel --no-default-features --features board-pine64 >/dev/null 2>&1 \
    || { echo "BOARD-PINE64 TEST FAILED (--features board-pine64 does not build)"; exit 1; }
echo "  board-pine64 feature build: OK"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4996,server,nowait \
    -qmp tcp:127.0.0.1:4997,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/board-pine64.out"
python3 scripts/shell-demo.py 4996 4997 "${TMPDIR:-/tmp}/board-pine64.png" \
    "echo warmup" "board" "board pine64" "board a64" "board pinephone" \
    "board list" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "board:|known boards|UART \(|GIC |RTC |GPIO |fw_cfg|virtio-mmio base|\^ Design|map only|full-featured|^ +(virt|raspi|rpi4|pine64) +(qemu|raspberry|pine)" "$OUT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md <<'PY'
import re, sys
s = open(sys.argv[1], "rb").read().decode("latin1")
doc = open(sys.argv[2], encoding="utf-8").read()

def norm(a):                              # 0x01C2_8000 -> 0x01c28000
    return a.replace("_", "").lower()

# DERIVED reference: pull the PINE64 (4th data column) cell out of the blueprint table.
def blueprint(row_label):
    m = re.search(r"^\|\s*" + row_label + r"\s*\|(.+)$", doc, re.M)
    assert m, f"blueprint row '{row_label}' not found in docs/15"
    return m.group(1).split("|")[2]       # QEMU virt | RPi4 | >PINE64<

def addr(cell, row):
    a = re.search(r"0x[0-9A-Fa-f_]+", cell)
    assert a, f"no address in PINE64 column of '{row}': {cell!r}"
    return norm(a.group(0))

uart_cell = blueprint("Primary UART")
uart = addr(uart_cell, "Primary UART")            # 0x01c28000
gicd = addr(blueprint("Interrupt ctrl"), "Interrupt ctrl")  # 0x01c81000
assert (uart, gicd) == ("0x01c28000", "0x01c81000"), (uart, gicd)
gicc = "0x%08x" % (int(gicd, 16) + 0x1000)        # A64 GICv2 CPU iface = dist + 0x1000

# The whole point of this board: the blueprint says its UART is NOT a PL011.
if "DesignWare" not in uart_cell or "PL011" in uart_cell:
    print(f"BOARD-PINE64 TEST FAILED (blueprint no longer calls the A64 UART a "
          f"DesignWare part: {uart_cell!r})"); sys.exit(1)

def row(label, value):                    # labels share a 16-column field
    return f"  {label:<16} {value}"

def need(*frags):
    for f in frags:
        if f not in s:
            print(f"BOARD-PINE64 TEST FAILED (missing '{f}')"); sys.exit(1)

# `board` (no arg): the ACTIVE virt map. Unchanged by the UART-label refactor —
# a PL011 board must still print exactly what it printed before this milestone.
need("board: qemu-virt (aarch64)",
     "UART (PL011)     @ 0x09000000",
     "GIC distributor  @ 0x08000000",
     "virtio-mmio base @ 0x0a000000",
     "(real boards define their own map; see docs/15)")

# `board pine64`: the A64 map, addresses matching the blueprint, absent peripherals
# printed as `none` (never as address 0).
need("board: pine64 (allwinner a64)",
     "not active",                                # inspection, not the booted board
     row("UART (8250dw)", f"@ {uart}"),
     row("GIC distributor", f"@ {gicd}"),
     row("GIC cpu iface", f"@ {gicc}"),
     row("RTC  (PL031)", "none"),
     row("GPIO", "@ 0x01c20800"),                 # PIO pin controller
     row("fw_cfg", "none"),
     row("virtio-mmio base", "none"),
     "map only:")
if "@ 0x00000000" in s:
    print("BOARD-PINE64 TEST FAILED (a missing peripheral printed as address 0)"); sys.exit(1)

# The caveat: this board needs a different UART *driver*, not just a different base.
need("^ DesignWare 8250/16550", "the PL011 driver does not apply")

# ...and the A64 block must never label its UART a PL011.
block = s.split("board: pine64 (allwinner a64)")[1][:800]
if "UART (PL011)" in block:
    print("BOARD-PINE64 TEST FAILED (A64 map labelled its UART a PL011)"); sys.exit(1)
if "8250dw" not in block:
    print("BOARD-PINE64 TEST FAILED (A64 map did not name the 8250 UART part)"); sys.exit(1)

# The `a64` / `pinephone` aliases must resolve to the same map (3 A64 blocks in all).
if s.count("board: pine64 (allwinner a64)") < 3:
    print("BOARD-PINE64 TEST FAILED (alias 'a64'/'pinephone' did not resolve to the "
          "A64 map)"); sys.exit(1)

# `board list` now covers all four boards, active one still marked.
need("known boards:", "virt   qemu-virt (aarch64) (active)",
     "raspi  raspberry-pi (bcm2837)", "rpi4   raspberry-pi-4 (bcm2711)",
     "pine64 pine64 (allwinner a64)")

print("BOARD-PINE64 TEST PASSED  (board-pine64 builds; A64 map "
      f"UART {uart} / GICv2 {gicd}+{gicc} matches docs/15; UART named 8250dw with the "
      "not-a-PL011 caveat; 4-board list + aliases OK)")
PY
