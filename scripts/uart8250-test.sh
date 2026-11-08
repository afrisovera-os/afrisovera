#!/usr/bin/env bash
# Verify the DesignWare 8250/16550 console driver added in kernel/src/uart_ns16550.rs
# (milestone 2.724 -- the DRIVER half of the 2.723 PINE64 map):
#   * `--features board-pine64` builds with the 8250 as its ACTIVE console, i.e. the
#     new driver really compiles into the console facade, not just alongside it;
#   * `uart8250` runs the driver against a RAM-backed fake register block and every
#     register-discipline check passes (offsets, THRE/DR polarity, bounded TX spin);
#   * the driver SELECTION is derived from the board map's UART part, so the same
#     virt build reports `pl011` for itself and `ns16550` for the A64 map;
#   * the blueprint still says the A64 needs its own driver behind the same interface.
# Self-checking: python re-derives the 16550 register offsets independently (reg-shift
# 2 => index i at i<<2) and asserts the kernel prints exactly those, and pulls the
# PINE64 UART base out of the docs/15 table rather than hardcoding it.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

# 1. The 8250 must compile as the *active* console on the board that has that part.
#    (On the default virt build the dispatch is a constant-false branch, so a broken
#    8250 driver could otherwise be optimized out of existence and never noticed.)
cargo build -p mansa-kernel --no-default-features --features board-pine64 >/dev/null 2>&1 \
    || { echo "UART8250 TEST FAILED (board-pine64 build with the 8250 console does not build)"; exit 1; }
echo "  board-pine64 build (8250 = active console): OK"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4856,server,nowait \
    -qmp tcp:127.0.0.1:4857,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/uart8250.out"
python3 scripts/shell-demo.py 4856 4857 "${TMPDIR:-/tmp}/uart8250.png" \
    "echo warmup" "uart8250" "board" "board pine64" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "uart8250: |\[ok\]|\[FAIL\]|console driver|UART \(" "$OUT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md <<'PY'
import re, sys
s = open(sys.argv[1], "rb").read().decode("latin1")
doc = open(sys.argv[2], encoding="utf-8").read()

def fail(msg):
    print(f"UART8250 TEST FAILED ({msg})"); sys.exit(1)

# --- INDEPENDENT REFERENCE -------------------------------------------------
# A 16550 has byte-wide registers at indices 0..7; a DesignWare integration maps
# them onto a 4-byte grid (device-tree `reg-shift = <2>`), so index i sits at i<<2.
# Derive the offsets here rather than copying them out of the kernel source.
SHIFT = 2
IDX = {"THR": 0, "IER": 1, "FCR": 2, "LCR": 3, "MCR": 4, "LSR": 5}
off = {name: i << SHIFT for name, i in IDX.items()}
assert (off["THR"], off["FCR"], off["LCR"], off["LSR"]) == (0x00, 0x08, 0x0c, 0x14)

# The kernel's own layout check must name exactly those offsets.
want_layout = ("layout: reg-shift %d -> THR +0x%02x, FCR +0x%02x, LCR +0x%02x, LSR +0x%02x"
               % (SHIFT, off["THR"], off["FCR"], off["LCR"], off["LSR"]))
if want_layout not in s:
    fail(f"driver did not report the reg-shift-{SHIFT} layout '{want_layout}'")

# --- SELF-TEST RESULT ------------------------------------------------------
m = re.search(r"uart8250: (\d+)/(\d+) checks passed", s)
if not m:
    fail("no 'uart8250: N/M checks passed' summary line")
passed, total = int(m.group(1)), int(m.group(2))
if total < 6:
    fail(f"self-test only ran {total} checks; expected the full register-discipline set")
if passed != total:
    fail(f"{total - passed} of {total} driver checks FAILED (see [FAIL] lines above)")
if "[FAIL]" in s:
    fail("a [FAIL] check line is present despite the passing summary")

# The checks that matter individually -- a green count means nothing if the driver
# stopped exercising the hang-proofing or the check-then-read RX protocol.
for frag in (
    "[ok]   init: FCR +0x08 = FIFO enable + rx/tx reset; LCR/IER/MCR untouched",
    "[ok]   tx: THRE set -> byte written to THR +0x00",
    "[ok]   tx: THRE clear -> bounded spin drops the byte (no hang, THR untouched)",
    "[ok]   rx: DR clear -> read_byte() = none",
    "[ok]   rx: DR set -> read_byte() = the byte in RBR +0x00",
):
    if frag not in s:
        fail(f"missing driver check '{frag}'")

# The bounded TX spin is the difference between a lost character and a dead kernel:
# the limit must be reported and must be finite and non-trivial.
m = re.search(r"tx spin limit (\d+)\)", s)
if not m:
    fail("self-test did not report the TX spin limit")
if not (1000 <= int(m.group(1)) <= 100_000_000):
    fail(f"implausible TX spin limit {m.group(1)}")

# --- DRIVER SELECTION IS DERIVED FROM THE BOARD MAP ------------------------
# Same binary, two answers: the active (PL011) board and the inspected A64 map.
if "uart8250: console driver on this build = pl011 (PL011 @ 0x09000000)" not in s:
    fail("virt build did not report pl011 as its own console driver")

def block_after(marker, n=900):
    i = s.find(marker)
    return s[i:i + n] if i >= 0 else ""

virt = block_after("board: qemu-virt (aarch64)")
a64 = block_after("board: pine64 (allwinner a64)")
if "console driver   pl011" not in virt:
    fail("`board` did not name pl011 as the virt board's console driver")
if "console driver   ns16550" not in a64:
    fail("`board pine64` did not name ns16550 as the A64's console driver")
if "ns16550" in virt.split("board: pine64")[0]:
    fail("the PL011 board was credited with the 8250 driver")

# The A64 map must still say the part is a DesignWare 8250 AND now name the driver.
if "8250dw" not in a64 or "UART (PL011)" in a64:
    fail("A64 map stopped naming the 8250 part")
if "uart_ns16550 drives it" not in a64:
    fail("A64 map does not say which module drives its UART")

# --- BLUEPRINT TIE (docs/15) -----------------------------------------------
row = re.search(r"^\|\s*Primary UART\s*\|(.+)$", doc, re.M)
if not row:
    fail("blueprint row 'Primary UART' not found in docs/15")
pine_cell = row.group(1).split("|")[2]
if "DesignWare" not in pine_cell or "PL011" in pine_cell:
    fail(f"blueprint no longer calls the A64 UART a DesignWare part: {pine_cell!r}")
base = re.search(r"0x[0-9A-Fa-f_]+", pine_cell).group(0).replace("_", "").lower()
if base != "0x01c28000":
    fail(f"blueprint A64 UART base changed to {base}; driver/map must follow")
if f"@ {base}" not in a64:
    fail(f"A64 map does not print the blueprint's UART base {base}")
# docs/15 item 1 asked for "a new driver behind the same `uart` interface, selected
# per board" -- the doc must still frame it that way for this test to mean anything.
if "behind the same `uart` interface" not in doc:
    fail("docs/15 no longer specifies a per-board driver behind the same uart interface")

print(f"UART8250 TEST PASSED  (board-pine64 builds with the 8250 as active console; "
      f"{passed}/{total} register-discipline checks pass at the independently derived "
      f"reg-shift-{SHIFT} offsets THR+0x00/FCR+0x08/LCR+0x0c/LSR+0x14; driver selection "
      f"derived from the map: virt->pl011, A64 {base}->ns16550)")
PY
