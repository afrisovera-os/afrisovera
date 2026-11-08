#!/usr/bin/env bash
# Verify `board check` (milestone 2.725) — the kernel diffing its COMPILE-TIME
# peripheral map (kernel/src/board.rs) against the DEVICE TREE it actually parsed
# (kernel/src/fdt.rs). docs/15-hardware-bringup.md item 4 / suggested-order step 2.
#
# Self-checking, and deliberately from two INDEPENDENT sources — neither of which is
# the kernel source being tested:
#   * the `map` column is re-derived from the QEMU-virt column of the blueprint table
#     in docs/15-hardware-bringup.md;
#   * the `device tree` column is re-derived by parsing kernel/src/virt.dtb with a
#     small FDT walker written here in python (magic, struct/strings offsets, node
#     tokens, `compatible`/`device_type`/`reg` at depth 2) — NOT copied from fdt.rs.
# The `tree model` line ('linux,dummy-virt') is the proof the tree column really came
# from the blob: that string exists nowhere in board.rs, so a `board check` that just
# echoed its own map back at itself could not produce it.
#
# Also asserted: the check names its source as the BUNDLED reference DTB (on QEMU's
# `-kernel` ELF path firmware passes x0=0, so this is a fixture, not the live
# machine — a self-check must not read as more than it is), and that a cross-machine
# build compiles the not-comparable branch.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

# 1. A non-virt board must still build: on those the bundled virt tree describes a
#    DIFFERENT machine, so check() takes its "not comparable" path (built first; the
#    default build below reclaims $BIN).
cargo build -p mansa-kernel --no-default-features --features board-rpi4 >/dev/null 2>&1 \
    || { echo "BOARD-CHECK TEST FAILED (--features board-rpi4 does not build)"; exit 1; }
echo "  cross-machine board build (board-rpi4): OK"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4858,server,nowait \
    -qmp tcp:127.0.0.1:4859,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/board-check.out"
python3 scripts/shell-demo.py 4858 4859 "${TMPDIR:-/tmp}/board-check.png" \
    "echo warmup" "board check" "dtb" "board" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "board check:|tree source|tree model|peripheral |UART base|UART part|GIC distributor|GIC cpu iface|RTC \(PL031\)|DRAM base|DRAM size|=> [0-9]+ agree|not comparable|only peripherals" "$OUT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md kernel/src/virt.dtb <<'PY'
import re, struct, sys
s = open(sys.argv[1], "rb").read().decode("latin1")
doc = open(sys.argv[2], encoding="utf-8").read()
dtb = open(sys.argv[3], "rb").read()

def fail(msg):
    print(f"BOARD-CHECK TEST FAILED ({msg})"); sys.exit(1)

# --- source A: the blueprint's QEMU-virt column -> what the MAP column must say ---
def blueprint(row_label):
    m = re.search(r"^\|\s*" + row_label + r"\s*\|(.+)$", doc, re.M)
    if not m: fail(f"blueprint row '{row_label}' not found in docs/15")
    col = m.group(1).split("|")[0]                 # >QEMU virt< | RPi4 | PINE64
    a = re.search(r"0x[0-9A-Fa-f_]+", col)
    if not a: fail(f"no address in the virt column of '{row_label}': {col!r}")
    return a.group(0).replace("_", "").lower(), col

map_uart, uart_cell = blueprint("Primary UART")    # 0x09000000
map_gicd, _         = blueprint("Interrupt ctrl")  # 0x08000000
map_gicc, _         = blueprint("GIC CPU iface")   # 0x08010000
map_rtc,  _         = blueprint("RTC")             # 0x09010000
map_ram,  ram_cell  = blueprint("Main DRAM")       # 0x40000000
if (map_uart, map_gicd, map_gicc, map_rtc, map_ram) != (
        "0x09000000", "0x08000000", "0x08010000", "0x09010000", "0x40000000"):
    fail(f"blueprint virt column moved: {(map_uart, map_gicd, map_gicc, map_rtc, map_ram)}")
if "PL011" not in uart_cell:
    fail(f"blueprint no longer calls the virt UART a PL011: {uart_cell!r}")
# DRAM is the only row with a size as well as a base (2.730), so the blueprint states
# the nominal figure the map declares and the kernel's map column must match it.
m = re.search(r"(\d+)\s*MiB", ram_cell)
if not m: fail(f"no DRAM size in the virt column of 'Main DRAM': {ram_cell!r}")
map_ram_mib = f"{m.group(1)} MiB"

# --- source B: an independent FDT walk of the reference blob -> the TREE column ---
FDT_MAGIC, BEGIN, END_NODE, PROP, NOP, END = 0xd00dfeed, 1, 2, 3, 4, 9
magic, _total, off_struct, off_strings = struct.unpack_from(">4I", dtb, 0)
if magic != FDT_MAGIC:
    fail(f"kernel/src/virt.dtb is not an FDT (magic {magic:#x})")

def cstr(buf, off):
    e = buf.index(b"\0", off)
    return buf[off:e].decode("latin1")

p, depth = off_struct, 0
model, tree = "", {}          # tree: part -> first base address
cur = {}
while True:
    (tok,) = struct.unpack_from(">I", dtb, p); p += 4
    if tok == BEGIN:
        depth += 1
        p += (len(cstr(dtb, p)) + 4) & ~3
        if depth == 2: cur = {}
    elif tok == END_NODE:
        if depth == 2 and "reg" in cur:
            for part in cur.get("parts", []):
                tree.setdefault(part, cur["reg"])
            # A GICv2 node states its CPU interface as the SECOND range of the same
            # `reg` — the property milestone 2.727 depends on. Walked here too, so the
            # expected value still comes from the blob rather than from fdt.rs.
            if "gic" in cur.get("parts", []) and "reg2" in cur:
                tree.setdefault("gic:cpu", cur["reg2"])
            # The `/memory` node (2.730) names itself with `device_type`, not with a
            # `compatible` — memory is not a device — and it is the one node whose SIZE
            # is read as well as its base.
            if cur.get("memory") and "regsize" in cur:
                tree.setdefault("memory", cur["reg"])
                tree.setdefault("memory:size", cur["regsize"])
        depth -= 1
    elif tok == PROP:
        ln, nameoff = struct.unpack_from(">2I", dtb, p); p += 8
        name, val = cstr(dtb, off_strings + nameoff), dtb[p:p + ln]
        if depth == 1 and name == "model":
            model = val.rstrip(b"\0").decode("latin1")
        elif depth == 2 and name == "compatible":
            parts = []
            if b"pl011" in val: parts.append("uart:PL011")
            if b"dw-apb-uart" in val or b"ns16550" in val: parts.append("uart:8250dw")
            if b"gic" in val: parts.append("gic")
            if b"pl031" in val: parts.append("rtc")
            cur["parts"] = parts
        elif depth == 2 and name == "device_type" and val.rstrip(b"\0") == b"memory":
            cur["memory"] = True
        elif depth == 2 and name == "reg" and ln >= 8:
            # root #address-cells is 2 on virt: first 8 bytes are the base address.
            cur["reg"] = "0x%08x" % struct.unpack_from(">Q", val, 0)[0]
            # ...and #size-cells is 2 as well, so the next 8 bytes are the length. Only
            # the memory node's is used: a peripheral window's size is not compared.
            if ln >= 16:
                cur["regsize"] = struct.unpack_from(">Q", val, 8)[0]
            # #address-cells + #size-cells = 4 cells = 16 bytes per range, so the
            # second range's address starts at byte 16.
            if ln >= 24:
                cur["reg2"] = "0x%08x" % struct.unpack_from(">Q", val, 16)[0]
        p += (ln + 3) & ~3
    elif tok == NOP:
        pass
    else:
        break

dt_uart_part = "PL011" if "uart:PL011" in tree else ("8250dw" if "uart:8250dw" in tree else "")
dt_uart = tree.get("uart:PL011") or tree.get("uart:8250dw") or "none"
dt_gicd = tree.get("gic", "none")
dt_gicc = tree.get("gic:cpu", "none")
dt_rtc  = tree.get("rtc", "none")
dt_ram  = tree.get("memory", "none")
if not model or dt_uart == "none" or dt_gicd == "none" or dt_rtc == "none":
    fail(f"independent DTB walk found too little: model={model!r} uart={dt_uart} "
         f"gic={dt_gicd} rtc={dt_rtc}")
if dt_gicc == "none":
    fail("independent DTB walk found no SECOND reg range on the GIC node — the CPU "
         "interface is what milestone 2.727 resolves; a tree without it cannot test it")
if dt_ram == "none":
    fail("independent DTB walk found no /memory node — DRAM is what 2.730 checks; "
         "a tree without it cannot test it")
sz = tree["memory:size"]
dt_ram_mib = f"{sz // (1024 * 1024)}{'+' if sz % (1024 * 1024) else ''} MiB"

def row(label, mapv, dtv, verdict="agree"):        # 16/12/12-column layout
    return f"  {label:<16} {mapv:<12} {dtv:<12} {verdict}"

def need(*frags):
    for f in frags:
        if f not in s:
            fail(f"missing '{f}'")

# The six compared rows: map column from docs/15, tree column from the blob. DRAM's
# BASE is one of them — memory starting where the map does not expect is a map error
# exactly as a wrong UART base is.
need("board check: qemu-virt (aarch64) vs device tree",
     row("UART base",       map_uart, dt_uart),
     row("UART part",       "PL011",  dt_uart_part),
     row("GIC distributor", map_gicd, dt_gicd),
     row("GIC cpu iface",   map_gicc, dt_gicc),
     row("RTC (PL031)",     map_rtc,  dt_rtc),
     row("DRAM base",       map_ram,  dt_ram))

# DRAM's SIZE is the row that is printed and not counted (2.730): the fixture was
# dumped at a different `-m` than the map declares, which is exactly the case this row
# exists to report. A size difference must read as a size, never as a map error.
if map_ram_mib == dt_ram_mib:
    fail("the fixture now states the map's own DRAM size — the 'differs' path is untested")
m = re.search(r"^  DRAM size\s+(\d+\+? MiB)\s+(\d+\+? MiB)\s+(.+)$", s, re.M)
if not m:
    fail("`board check` prints no DRAM size row")
if (m.group(1), m.group(2)) != (map_ram_mib, dt_ram_mib):
    fail(f"`board check` DRAM size row is {m.group(1)} vs {m.group(2)}, expected "
         f"{map_ram_mib} (docs/15) vs {dt_ram_mib} (virt.dtb)")
if "a size, not a map error" not in m.group(3):
    fail(f"a differing DRAM size was reported as {m.group(3)!r}, not as a size")

# The tree column is genuinely parsed, not echoed from board.rs — the model string
# only exists in the blob.
need(f"tree model       '{model}'")

# Every counted row agreed, and the summary must say so in those words.
need("=> 6 agree, 0 disagree (0 rows the tree does not describe)")
if "DISAGREE" in s:
    fail("a peripheral disagreed with the device tree on virt")
if "not comparable" in s:
    fail("virt build called its own virt tree cross-machine")

# Honesty: this ran against the BUNDLED fixture (QEMU -kernel passes x0=0), and the
# check must say that rather than implying it read the live machine.
need("tree source      bundled reference virt.dtb (firmware passed no pointer)")
if "live blob from firmware" in s:
    fail("claimed a live DTB on the ELF boot path, where firmware passes none")

# `board` itself is untouched by this milestone.
need("board: qemu-virt (aarch64)", "UART (PL011)     @ 0x09000000", "console driver   pl011")

print(f"BOARD-CHECK TEST PASSED  (map from docs/15 vs an independent walk of "
      f"virt.dtb: UART {dt_uart} {dt_uart_part} / GIC {dt_gicd}+{dt_gicc} / "
      f"RTC {dt_rtc} / DRAM {dt_ram}, model '{model}', 6 agree 0 disagree; the "
      f"fixture's {dt_ram_mib} against the map's {map_ram_mib} is reported as a size, "
      f"not as a map error; source named as the bundled fixture)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "BOARD-CHECK TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
