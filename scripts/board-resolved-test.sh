#!/usr/bin/env bash
# Verify `board resolved` (milestone 2.726) — the OVERRIDE half of docs/15 item 4 /
# suggested-order step 2. Milestone 2.725 made the kernel *compare* its compile-time
# peripheral map with the device tree; this one lets a LIVE tree win over the map for
# peripheral consumers, under four rules that keep the override honest:
#
#   1. only a live blob may override (the bundled reference virt.dtb is a fixture and
#      must never drive real MMIO);
#   2. a peripheral the walker did not find (base 0) never zeroes a working address —
#      silence is not absence;
#   3. a base may not move to a tree node naming a DIFFERENT UART part (a right
#      address with the wrong driver is a dead console);
#   4. the console itself stays compile-time — it is brought up before the tree is
#      parsed, so a bad blob must still have somewhere to report itself;
#   5. the GIC's two windows move together or not at all (2.727);
#   6. a tree that names a DIFFERENT machine overrides nothing at all (2.729) — the
#      only rule that judges the blob as a whole rather than one peripheral row;
#   7. an IMPOSSIBLE /memory claim is refused rather than believed (2.730) — the only
#      rule that asks whether a value is possible at all, because DRAM is the only row
#      whose consumer is the frame allocator: a wrong MMIO base costs one device, a
#      wrong RAM range hands out addresses that are not memory. A merely *different*
#      size is not impossible (`-m 384M` and a 2 GiB SKU are the machine being itself)
#      and is taken.
#
# Two independent things are checked, from sources that are not the code under test:
#   * the RESOLVED values on this machine, re-derived from the QEMU-virt column of the
#     blueprint table in docs/15-hardware-bringup.md (they must equal the map, because
#     QEMU's -kernel ELF path passes x0=0 — there is no live tree to override with);
#   * the seven RULES, exercised by `board resolved test`, which runs the pure
#     `resolve_from()` against SYNTHETIC trees. That is the only way to execute the
#     override branch at all on a machine that never receives a live DTB — without it
#     this milestone would ship a code path nobody has ever run.
# Also asserted: the real consumers (vfs mtime stamps, rng seed, the Device Manager
# entries) read the resolved base rather than the raw constant, so "the tree drives
# the drivers" is wiring and not just a printout.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

# A board whose UART part is NOT a PL011 must still compile the resolver (its console
# line resolves to a different driver); built first, the default build reclaims $BIN.
cargo build -p mansa-kernel --no-default-features --features board-pine64 >/dev/null 2>&1 \
    || { echo "BOARD-RESOLVED TEST FAILED (--features board-pine64 does not build)"; exit 1; }
echo "  non-PL011 board build (board-pine64): OK"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4860,server,nowait \
    -qmp tcp:127.0.0.1:4861,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/board-resolved.out"
python3 scripts/shell-demo.py 4860 4861 "${TMPDIR:-/tmp}/board-resolved.png" \
    "echo warmup" "board resolved" "board resolved test" "devices" "board check" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "board resolved|tree source|peripheral |UART base|UART part|GIC distributor|GIC cpu iface|RTC \(PL031\)|DRAM base|DRAM size|heap still|console still|\[ok\]|\[FAIL\]|rules held|pl031-rtc|gicv2" "$OUT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md kernel/src/vfs.rs kernel/src/rng.rs <<'PY'
import re, sys
s   = open(sys.argv[1], "rb").read().decode("latin1")
doc = open(sys.argv[2], encoding="utf-8").read()
vfs = open(sys.argv[3], encoding="utf-8").read()
rng = open(sys.argv[4], encoding="utf-8").read()

def fail(msg):
    print(f"BOARD-RESOLVED TEST FAILED ({msg})"); sys.exit(1)

def need(*frags):
    for f in frags:
        if f not in s:
            fail(f"missing '{f}'")

# --- the expected values come from the blueprint, not from board.rs ---------------
def blueprint(row_label):
    m = re.search(r"^\|\s*" + row_label + r"\s*\|(.+)$", doc, re.M)
    if not m: fail(f"blueprint row '{row_label}' not found in docs/15")
    col = m.group(1).split("|")[0]                 # >QEMU virt< | RPi4 | PINE64
    a = re.search(r"0x[0-9A-Fa-f_]+", col)
    if not a: fail(f"no address in the virt column of '{row_label}': {col!r}")
    return a.group(0).replace("_", "").lower(), col

uart, uart_cell = blueprint("Primary UART")    # 0x09000000
gicd, _         = blueprint("Interrupt ctrl")  # 0x08000000
gicc, _         = blueprint("GIC CPU iface")   # 0x08010000
rtc,  _         = blueprint("RTC")             # 0x09010000
ram,  ram_cell  = blueprint("Main DRAM")       # 0x40000000
if (uart, gicd, gicc, rtc, ram) != ("0x09000000", "0x08000000", "0x08010000",
                                    "0x09010000", "0x40000000"):
    fail(f"blueprint virt column moved: {(uart, gicd, gicc, rtc, ram)}")
if "PL011" not in uart_cell:
    fail(f"blueprint no longer calls the virt UART a PL011: {uart_cell!r}")
# DRAM is the one row with a size as well as a base, and the size is what the map
# nominally declares — so the blueprint states it and the kernel must agree.
m = re.search(r"(\d+)\s*MiB", ram_cell)
if not m: fail(f"no DRAM size in the virt column of 'Main DRAM': {ram_cell!r}")
ram_mib = f"{m.group(1)} MiB"
if ram_mib != "256 MiB":
    fail(f"blueprint virt DRAM size moved: {ram_mib} (this script boots QEMU at -m 256M)")

# --- part 1: what is in use on THIS machine --------------------------------------
# QEMU's ELF boot passes no DTB, so fdt.rs falls back to the bundled fixture and
# rule 1 must leave every row on the compile-time map.
need("board resolved: qemu-virt (aarch64)",
     "tree source      bundled reference virt.dtb (a fixture never drives MMIO)")
if "live blob from firmware" in s.split("board resolved test")[0]:
    fail("claimed a live DTB on the ELF boot path, where firmware passes none")

def row(label, mapv, use, src):                    # 16-col label, 12-col value cols
    return f"  {label:<16} {mapv:<12} {use:<12} {src}"

need(row("UART base",       uart, uart, "map (compile-time)"),
     row("GIC distributor", gicd, gicd, "map (compile-time)"),
     row("GIC cpu iface",   gicc, gicc, "map (compile-time)"),
     row("RTC (PL031)",     rtc,  rtc,  "map (compile-time)"),
     # DRAM (2.730): base and size are one range and share one source word, and on
     # this boot that word must be the map — the fixture may not size our memory.
     row("DRAM base",       ram,     ram,     "map (compile-time)"),
     row("DRAM size",       ram_mib, ram_mib, "map (compile-time)"),
     "  UART part        PL011")
# A refusal (rule 7) must never be claimed on a boot where nothing was even offered.
if "/memory claim was REFUSED" in s.split("board resolved test")[0]:
    fail("reported a rule-7 DRAM refusal on a boot with no live tree to refuse")
if "OVERRIDE" in s.split("board resolved test")[0]:
    fail("a fixture DTB overrode the map (rule 1 violated on the live machine)")
# Rule 6's refusal must not be claimed here: the fixture describes the very machine
# this build targets, so the honest reason nothing moved is rule 1, not rule 6.
if "REFUSED: another machine" in s.split("board resolved test")[0]:
    fail("called the bundled virt fixture another machine's tree on a virt build")

# Rule 4 must be stated on every listing, not just implied.
need(f"console still uses the compile-time base {uart} (PL011 via pl011)")
# And the same admission for the heap (2.730): it is sized before the tree is parsed,
# so a resolved DRAM range that no allocator consumes yet must say so rather than
# leave the operator to assume the memory listing drives anything.
need("heap still sized at compile time")

# --- part 2: the seven rules, against synthetic trees ------------------------------
rules = s.split("board resolved: rule self-test")[-1]
for phrase in ("no tree: every value stays on the compile-time map",
               "rule 1: bundled reference tree never overrides (fixture != machine)",
               "live tree agreeing with the map reads as confirmed, not as an override",
               "live tree overrides GIC + RTC bases the map got wrong",
               "rule 2: a tree silent on a peripheral leaves that row on the map",
               "rule 3: UART part mismatch blocks the base override (no dead console)",
               # Rule 5 (milestone 2.727): the GIC's two windows are one controller.
               "rule 5: both GIC windows named - the pair moves together",
               "rule 5: distributor alone is a PARTIAL discovery - neither window moves",
               "rule 5: CPU interface alone is refused the same way (symmetric)",
               "rule 5: a pair may be half-confirmed half-overridden, but never half-taken",
               # Rule 6 (milestone 2.729): a tree may only speak for the machine it
               # names. The refusal case and the two that keep it from over-reaching.
               "rule 6: another machine's tree overrides nothing, however well-formed",
               "rule 6: the same tree naming THIS machine still overrides (identity, not content)",
               "rule 6: a tree naming NO machine is not a refusal - resolution proceeds",
               "rule 6: a foreign FIXTURE is refused as a fixture (rule 1 first), not as a machine",
               # Rule 7 (milestone 2.730): DRAM. The first four cases are the rules
               # above reaching a new row (silence, override, confirmation), the next
               # four are the impossible claims, and the last two check that rules 1
               # and 6 bound this row as they bound the peripherals.
               "rule 2: a tree with no /memory node leaves DRAM on the map (size 0 = silence)",
               "rule 7: a plausible /memory range overrides the map's nominal DRAM size",
               "rule 7: a tree stating the map's own DRAM range reads as confirmed",
               "rule 7: DRAM smaller than the resident kernel is REFUSED, not believed",
               "rule 7: DRAM beyond the 39-bit identity map is REFUSED",
               "rule 7: a DRAM range that wraps the address space is REFUSED",
               "rule 7: a DRAM base that is not page-aligned is REFUSED",
               "rule 1: a fixture tree may not size our DRAM (and that is not a rule-7 refusal)",
               "rule 6: another machine's tree may not size our DRAM either"):
    if f"[ok]   {phrase}" not in rules:
        fail(f"rule not proved: {phrase}")
if "[FAIL]" in rules:
    fail("a resolution rule did not hold")
need("board resolved: 23/23 rules held")

# --- part 3: the consumers are really wired to resolved(), not to the constant ----
if "resolved().rtc" not in vfs:
    fail("vfs.rs still stamps mtimes from the raw RTC constant, not the resolved base")
if "resolved().rtc" not in rng:
    fail("rng.rs still seeds from the raw RTC constant, not the resolved base")
# The Device Manager registers what is in USE; on this machine that equals the map.
m = re.search(r"^pl031-rtc\s+\w+\s+(0x[0-9a-f]+)", s, re.M)
if not m or int(m.group(1), 16) != int(rtc, 16):
    fail(f"devices lists pl031-rtc at {m.group(1) if m else '?'} , expected {rtc}")
m = re.search(r"^gicv2\s+\w+\s+(0x[0-9a-f]+)", s, re.M)
if not m or int(m.group(1), 16) != int(gicd, 16):
    fail(f"devices lists gicv2 at {m.group(1) if m else '?'} , expected {gicd}")

# --- part 4: 2.725's `board check` is undisturbed by the override layer -----------
# Six counted rows now: DRAM *base* is checked like any other address, since memory
# starting where the map does not expect is a map error exactly as a wrong UART base
# is. The DRAM *size* row is the one that is printed and not counted.
need("=> 6 agree, 0 disagree (0 rows the tree does not describe)")
chk = s.split("board check")[-1]
m = re.search(r"^  DRAM base\s+(0x[0-9a-f]+)\s+(0x[0-9a-f]+)\s+(\S+)", chk, re.M)
if not m or m.group(1) != ram or m.group(3) != "agree":
    fail(f"`board check` DRAM base row is {m.groups() if m else '(missing)'} , expected {ram} agree")
m = re.search(r"^  DRAM size\s+(\d+\+? MiB)\s+(\d+\+? MiB)\s+(.+)$", chk, re.M)
if not m:
    fail("`board check` prints no DRAM size row")
if m.group(1) != ram_mib:
    fail(f"`board check` states the map's DRAM as {m.group(1)}, blueprint says {ram_mib}")
# The fixture is dumped at a different -m than the map declares, which is exactly the
# case this row exists to report: a size difference must read as a size, never as a
# map error, and must not move the counters above.
if m.group(2) == ram_mib:
    fail("the fixture now states the map's own DRAM size — the 'differs' path is untested")
if "a size, not a map error" not in m.group(3):
    fail(f"a differing DRAM size was reported as {m.group(3)!r}, not as a size")

print(f"BOARD-RESOLVED TEST PASSED  (in-use map from docs/15: UART {uart} / GIC "
      f"{gicd}+{gicc} / RTC {rtc} / DRAM {ram} {ram_mib}, all sourced "
      f"'map (compile-time)' because the fixture may not override; 23/23 override "
      f"rules held against synthetic trees, rule 7 included: impossible memory is "
      f"refused while a merely different size is taken; the fixture's own differing "
      f"size is reported as a size, not as a map error; vfs+rng+devices read the "
      f"resolved base; console and heap stay compile-time)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "BOARD-RESOLVED TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
