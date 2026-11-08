#!/usr/bin/env bash
# Verify `gicbase` (milestone 2.727) — the resolved map reaching the INTERRUPT driver.
#
# Milestone 2.726 let a live device tree override the compile-time peripheral map, but
# `gic.rs` still derived `const GICD_BASE`/`GICC_BASE` from the map, so the override
# reached the RTC readers and the device listing and stopped exactly at the one
# peripheral whose wrong address is silent: an interrupt controller at the wrong
# address does not report anything, it simply never interrupts. 2.727 makes both GIC
# windows come from `board::resolved()` at use time, which buys correctness at the
# price of an ORDERING requirement — `board::resolve()` must precede every GIC access,
# on every core.
#
# Three things are checked, none of them from the code under test:
#   Part A (-smp 1, interactive): the bases `gic.rs` is actually driving, re-derived
#     from the QEMU-virt column of the blueprint table in docs/15-hardware-bringup.md,
#     with their provenance; plus the boot-order audit gic.rs keeps — every access
#     counted, and separately counted if it happened while the map was unresolved.
#     A printed address only proves the value; the counters prove the *timing*, which
#     is the actual claim of this milestone and the thing a comment cannot establish.
#   Part B (-smp 2, boot log): the secondary core really does run `gic::init_secondary`
#     (so the per-core path the audit counts is not dead code), and main.rs orders
#     `board::resolve()` before both `gic::init()` and `bring_up_secondary()` — a
#     source-level ordering assertion that fails loudly if a future edit hoists GIC
#     bring-up above resolution on a machine that has no live tree to expose it.
#   Part C (source): the compile-time constants are GONE from gic.rs, and the
#     inspectors (`gicinfo`/`giccinfo`) read the driver's base rather than the raw
#     constant — an inspector that reads a different address than the driver drives is
#     worse than no inspector.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

# --- Part A: interactive, single core -------------------------------------------
# Single core on purpose: driving the shell under -smp 2 is unreliable (core 1's
# async lines interleave with the prompt sync), which is why Part B reads a boot log.
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4862,server,nowait \
    -qmp tcp:127.0.0.1:4863,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
A="${TMPDIR:-/tmp}/gicbase-a.out"
python3 scripts/shell-demo.py 4862 4863 "${TMPDIR:-/tmp}/gicbase.png" \
    "echo warmup" "gicbase" "gicinfo" "giccinfo" "gicbase" > "$A" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

echo "[A] -smp 1 interactive gicbase:"
grep -aE "gicbase:|distributor |cpu interface|board::resolve\(\) ran|accesses |secondary cpu-iface|=> ORDER|gicinfo: |giccinfo: " "$A" | sed 's/^/    /'

# --- Part B: two cores, boot log only -------------------------------------------
B="${TMPDIR:-/tmp}/gicbase-b.log"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 2 -m 256M -nographic -kernel "$BIN" \
    >"$B" 2>&1 &
QPID=$!
# 2.905: the 12 s window was a CEILING, not a measurement - Part B's verdict is the
# THREE lines the grep below asserts, so wait for that set and stop paying the
# slow-host margin on a fast host. Cap 60 s; on timeout this returns 1 WITHOUT
# exiting, so the grep below is still what fails and still names the missing line.
# Patterns use []] rather than a backslash-] so afrish_marker_missing_init keeps the
# awk single-fork path (a backslash anywhere disables it) - 2.783's fork-cost finding.
afrish_wait_markers "$B" 60 \
    'core 1[]] online' 'core 1[]] scheduler up' 'SMP: 2 cores online'
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
echo "[B] -smp 2 secondary GICC bring-up:"
grep -aE "core 1\] online|core 1\] scheduler up|SMP: 2 cores online" "$B" | sed 's/^/    /'

python3 - "$A" "$B" docs/15-hardware-bringup.md kernel/src/gic.rs kernel/src/shell.rs \
                    kernel/src/main.rs <<'PY'
import re, sys
a     = open(sys.argv[1], "rb").read().decode("latin1")
b     = open(sys.argv[2], "rb").read().decode("latin1")
doc   = open(sys.argv[3], encoding="utf-8").read()
gic   = open(sys.argv[4], encoding="utf-8").read()
shell = open(sys.argv[5], encoding="utf-8").read()
main  = open(sys.argv[6], encoding="utf-8").read()

def fail(msg):
    print(f"GICBASE TEST FAILED ({msg})"); sys.exit(1)

def need(hay, *frags):
    for f in frags:
        if f not in hay:
            fail(f"missing '{f}'")

# --- the expected addresses come from the blueprint, not from board.rs ------------
def blueprint(row_label):
    m = re.search(r"^\|\s*" + row_label + r"\s*\|(.+)$", doc, re.M)
    if not m: fail(f"blueprint row '{row_label}' not found in docs/15")
    col = m.group(1).split("|")[0]                 # >QEMU virt< | RPi4 | PINE64
    x = re.search(r"0x[0-9A-Fa-f_]+", col)
    if not x: fail(f"no address in the virt column of '{row_label}': {col!r}")
    return x.group(0).replace("_", "").lower()

gicd = blueprint("Interrupt ctrl")   # 0x08000000
gicc = blueprint("GIC CPU iface")    # 0x08010000
if (gicd, gicc) != ("0x08000000", "0x08010000"):
    fail(f"blueprint virt GIC column moved: {(gicd, gicc)}")

# --- Part A.1: the bases in use, and where they came from -------------------------
# QEMU's -kernel ELF path passes x0=0, so there is no live tree and rule 1 keeps both
# windows on the map. The point is that they are *resolved* values that happen to
# equal the map, printed next to the map for comparison — not the map read directly.
need(a, "gicbase: GICv2 bases in use by gic.rs (resolved, not compiled in)",
        f"  distributor      {gicd}   map {gicd}   map (compile-time)",
        f"  cpu interface    {gicc}   map {gicc}   map (compile-time)")
if "partial tree discovery" in a:
    fail("reported a partial GIC discovery on a machine with no live tree at all")

# --- Part A.2: the ordering audit — the actual claim of this milestone -------------
need(a, "  board::resolve() ran = yes",
        "  => ORDER OK: every GIC access, on every core, used a resolved base")
if "ORDER VIOLATION" in a:
    fail("gic.rs recorded a GIC access made before board::resolve() ran")

m = re.search(r"accesses (\d+) \((\d+) before resolve\)", a)
if not m: fail("no GIC access tally printed")
uses, early = int(m.group(1)), int(m.group(2))
if uses == 0:
    fail("0 GIC accesses counted — the counters are not on the path the driver uses")
if early != 0:
    fail(f"{early} GIC access(es) used an unresolved base")

m = re.search(r"secondary cpu-iface inits (\d+) \((\d+) before resolve\)", a)
if not m: fail("no secondary cpu-interface tally printed")
sec, sec_early = int(m.group(1)), int(m.group(2))
if sec != 0:
    fail(f"{sec} secondary GICC init(s) under -smp 1, where there is no second core")
if sec_early != 0:
    fail("a secondary core initialised its CPU interface before resolution")

# The tally must be a live counter, not a constant: `gicbase` was run twice with
# `gicinfo`/`giccinfo` in between, and those inspectors now go through the driver's
# base accessors, so the second reading must be strictly higher than the first.
tallies = [int(x) for x in re.findall(r"accesses (\d+) \(", a)]
if len(tallies) < 2:
    fail("expected two gicbase readings")
if tallies[1] <= tallies[0]:
    fail(f"access count did not advance across gicinfo/giccinfo ({tallies}) — the "
         f"inspectors are not going through the resolved base accessors")

# --- Part A.3: the resolved base is a real, working GIC ---------------------------
# Reading GICD_TYPER/GICC_IIDR through the *resolved* address proves it is not merely
# a printed number: a wrong base would read back garbage or fault.
if not re.search(r"GICD_TYPER = 0x[0-9a-f]{8}", a):
    fail("gicinfo read no GICD_TYPER through the resolved distributor base")
if "distributor enabled" not in a:
    fail("the distributor at the resolved base is not enabled")
if not re.search(r"GICC_IIDR = 0x[0-9a-f]{8}", a):
    fail("giccinfo read no GICC_IIDR through the resolved CPU-interface base")

# --- Part B: the per-core path is real, and resolution precedes it ----------------
need(b, "core 1] online", "core 1] scheduler up", "SMP: 2 cores online")

def line_of(pat, what):
    m = re.search(pat, main, re.M)
    if not m: fail(f"main.rs no longer calls {what}")
    return main[:m.start()].count("\n") + 1

l_resolve = line_of(r"^\s*board::resolve\(\);", "board::resolve()")
l_gicinit = line_of(r"^\s*gic::init\(\);", "gic::init()")
l_smp     = line_of(r"^\s*bring_up_secondary\(\);", "bring_up_secondary()")
if not (l_resolve < l_gicinit):
    fail(f"main.rs calls gic::init() (line {l_gicinit}) before board::resolve() "
         f"(line {l_resolve}) — the distributor would come up on an unresolved base")
if not (l_resolve < l_smp):
    fail(f"main.rs starts the secondary (line {l_smp}) before board::resolve() "
         f"(line {l_resolve}) — core 1 would run its CPU interface unresolved")

# --- Part C: the compile-time constants are gone, and the inspectors agree ---------
if re.search(r"^\s*const GIC[DC]_BASE", gic, re.M):
    fail("gic.rs still derives a const GICD_BASE/GICC_BASE from the compile-time map")
if "crate::board::resolved().gic_dist" not in gic or "crate::board::resolved().gic_cpu" not in gic:
    fail("gic.rs does not read both windows from board::resolved()")
for accessor in ("crate::gic::dist_base()", "crate::gic::cpu_base()"):
    if accessor not in shell:
        fail(f"shell.rs inspectors do not use {accessor}")
# No MMIO *read* in shell.rs may still be aimed at the raw constant. Printing the
# constant is fine (the map listing and `board check`'s comparison rows exist to show
# it) — dereferencing it is not, because that is the inspector and the driver reading
# two different addresses.
for m in re.finditer(r"let base = crate::board::GIC_(DIST|CPU)_BASE", shell):
    fail("a shell inspector still dereferences the compile-time GIC constant "
         f"({m.group(0)}) instead of the driver's resolved base")

print(f"GICBASE TEST PASSED  (gic.rs drives GICD {gicd} + GICC {gicc}, both resolved "
      f"and both sourced 'map (compile-time)' from docs/15; {uses}->{tallies[1]} "
      f"accesses counted, 0 before resolve, 0 secondary inits under -smp 1; core 1 "
      f"brings up its own CPU interface under -smp 2 and main.rs resolves "
      f"(line {l_resolve}) before gic::init() ({l_gicinit}) and bring_up_secondary() "
      f"({l_smp}); no GICD_BASE/GICC_BASE consts remain)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "GICBASE TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
