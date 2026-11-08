#!/usr/bin/env bash
# Verify `frame` / `frame test` (milestone 2.731) — the CONSUMER half of the rules
# milestones 2.726–2.730 built.
#
# 2.730 resolved DRAM under seven rules (a fixture may not size our memory, another
# machine's tree overrides nothing, an impossible /memory claim is refused) and then
# admitted that nothing consumed the result: `main.rs` still sized the frame allocator
# from a RAW `fdt::ram_end_or()` read, so the largest consumer on the board walked
# around every one of those rules. This milestone feeds it `board::resolved()` instead,
# and makes the allocator's two truncations speak: the identity-map ceiling and the
# static bitmap both shrink whatever range they are given, and both used to do it in
# silence — a `-m 384M` boot reported "256 MiB" and read exactly like a machine that
# has 256 MiB. A bound the operator cannot see is indistinguishable from memory that is
# not there.
#
# Checked from sources that are NOT frame.rs:
#   * the DRAM base and size come from the blueprint table in docs/15-hardware-bringup.md;
#   * the identity-map ceiling is read out of kernel/src/paging.rs — the file that
#     BUILDS the map — so "the allocator stops where translation stops" is checked
#     against the mapping code rather than against a number frame.rs copied;
#   * the WIRING is asserted in the source: main.rs must reach the allocator through
#     `board::resolved()`, and `fdt::ram_end_or` — the shortcut around the rules — must
#     no longer exist for anyone to reach for;
#   * `mem` and `pmap` must report the same frame count as `frame`, so the new listing
#     describes the live allocator and is not a second, prettier opinion;
#   * the bounds themselves are exercised by `frame test` against SYNTHETIC ranges.
#     This machine boots with one range, and it is a range both bounds accept — without
#     the self-test, "the allocator refuses to hand out unmapped memory" would be a
#     claim about code that has never run.
#
# Two QEMU runs, for the reason smpinfo-test.sh has two: the interactive serial is a
# TCP server the demo joins ~8 s in, by which time the boot log has already been
# printed to nobody. Part A drives the shell over TCP; Part B re-boots under
# -nographic to a file so the boot line itself can be read and held to the same
# numbers the shell reports.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "FRAME-BOUNDS TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4864,server,nowait \
    -qmp tcp:127.0.0.1:4865,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/frame-bounds.out"
python3 scripts/shell-demo.py 4864 4865 "${TMPDIR:-/tmp}/frame-bounds.png" \
    "echo warmup" "frame" "frame test" "mem" "pmap" "pmap test" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

# --- Part B: the boot log, which the TCP client above joined too late to see -------
BOOT="${TMPDIR:-/tmp}/frame-bounds-boot.log"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M -nographic -kernel "$BIN" \
    >"$BOOT" 2>&1 &
BPID=$!
# 2.907: the 10 s window was a CEILING, not a measurement. Wait for Part B's own
# output instead, cap 60 s; on timeout this returns 1 WITHOUT exiting, so the grep
# and the python below are still what fail and still name the missing line.
# TWO markers, and the SECOND one is kept on MEASURED grounds (falsifier 2.921),
# NOT on the grounds first written here at 2.907. Part 5's bound check is
#   `"! bounded" in boot.split("[mansa] frame allocator")[-1][:400]`
# — an ABSENCE test over the 400 bytes AFTER the allocator line. The ORIGINAL
# rationale claimed one marker would kill QEMU before an ADJACENT `! bounded`
# (main.rs:883, the very next println) could reach the log. That claim is
# REFUTED: afrish_wait_markers POLLS, so a line already on disk when a poll fires
# is read in the SAME poll, and at real-guest adjacency ONE marker ALREADY
# catches the bound (falsify-2921 arm5a). The regime where the window really does
# truncate is a WIDENED GAP — `! bounded` landing more than a poll interval after
# the allocator line (guest scheduling, host load, an unflushed serial buffer).
# At a 6 s gap one marker DOES truncate and part 5 would print a silent PASS
# (arm5b); two markers close exactly that regime (arm5c). So the second marker
# buys FLUSH MARGIN, not adjacency safety: `Phase 0 deliverable met` is printed
# ~2 KB later and unconditionally, so the whole 400-byte window is on disk before
# the kill however late the bound line arrives.
# Patterns use [[] rather than a backslash-[ so afrish_marker_missing_init keeps
# the awk single-fork path (any backslash disables it) - 2.783's fork-cost finding.
afrish_wait_markers "$BOOT" 60 \
    '[[]mansa] frame allocator: ' \
    'Phase 0 deliverable met'
kill "$BPID" 2>/dev/null
wait "$BPID" 2>/dev/null

grep -aE "frame allocator|resolved DRAM|managed |below the base|bitmap holds|identity map   ends|identity map   Normal|BOUNDED|every frame|frames: |phys frames|\[ok\]|\[FAIL\]|bounds held|window checks held" "$OUT" | sed 's/^/  /'
grep -aE "^\[mansa\] +(frame allocator|! bounded)" "$BOOT" | sed 's/^/  /'

python3 - "$OUT" docs/15-hardware-bringup.md kernel/src/paging.rs kernel/src/main.rs kernel/src/fdt.rs "$BOOT" <<'PY'
import re, sys
# THE CONSOLE SPEAKS UTF-8, so the capture is read as UTF-8. Decoding it as latin1 —
# which every suite in this tree does, this one included until now — is byte-faithful
# and never throws, which is why it was chosen and why it went unnoticed: it is also
# WRONG for every line the kernel prints containing a non-ASCII character, and the
# kernel prints em dashes freely (paging.rs:376 among ~200 other kprintln! sites). A
# latin1 decode turns each one into three mojibake characters, so an assertion written
# against the line the kernel actually prints cannot match it, and the only way to
# assert such a line is to stop the pattern short of the em dash — which is what the
# board-window check below used to do, checking less than it meant to. `errors=replace`
# keeps the never-throws property (a capture cut mid-character by a killed QEMU is
# routine here) without silently corrupting the text around the cut.
s      = open(sys.argv[1], "rb").read().decode("utf-8", "replace")
doc    = open(sys.argv[2], encoding="utf-8").read()
paging = open(sys.argv[3], encoding="utf-8").read()
main   = open(sys.argv[4], encoding="utf-8").read()
fdt    = open(sys.argv[5], encoding="utf-8").read()
boot   = open(sys.argv[6], "rb").read().decode("utf-8", "replace")   # same reason as `s`

def fail(msg):
    print(f"FRAME-BOUNDS TEST FAILED ({msg})"); sys.exit(1)

def need(*frags):
    for f in frags:
        if f not in s:
            fail(f"missing '{f}'")

FRAME_SIZE = 4096

# --- the expected DRAM range comes from the blueprint, not from board.rs ----------
m = re.search(r"^\|\s*Main DRAM\s*\|(.+)$", doc, re.M)
if not m:
    fail("blueprint row 'Main DRAM' not found in docs/15")
cell = m.group(1).split("|")[0]                       # >QEMU virt< | RPi4 | PINE64
a = re.search(r"0x[0-9A-Fa-f_]+", cell)
z = re.search(r"(\d+)\s*MiB", cell)
if not a or not z:
    fail(f"no base/size in the virt column of 'Main DRAM': {cell!r}")
ram_base = int(a.group(0).replace("_", ""), 16)
ram_mib  = int(z.group(1))
if (ram_base, ram_mib) != (0x4000_0000, 256):
    fail(f"blueprint virt DRAM moved: {ram_base:#x} {ram_mib} MiB "
         f"(this script boots QEMU at -m 256M)")
ram_end = ram_base + ram_mib * 1024 * 1024

# --- the identity-map ceiling comes from the file that BUILDS the map -------------
# 2.733: it is no longer a per-board `#[cfg]` constant but the board's own declared DRAM
# rounded out to whole 1 GiB blocks, so the expected ceiling is computed here the same
# way — from the blueprint figures parsed above — and paging.rs must state it in those
# terms rather than in a literal.
if not re.search(r"pub const NORMAL_MAP_END: usize = NORMAL\.end\(\);", paging):
    fail("paging.rs no longer derives NORMAL_MAP_END from its Normal window — the "
         "allocator's ceiling would be a number frame.rs invented rather than one the "
         "mapping code enforces")
if not re.search(r"pub const NORMAL: Window = window\(\s*crate::board::ACTIVE\.ram_base,"
                 r"\s*crate::board::ACTIVE\.ram_size,?\s*\)", paging):
    fail("the Normal window is not built from board::ACTIVE's declared DRAM (2.733)")
if re.search(r"l1\.entries\[\d+\][^\n]*ATTR_IDX_NORMAL", paging):
    fail("paging.rs still writes a Normal block at a hard-coded index — the window is "
         "back to being a constant that happens to suit virt")
GIB = 1 << 30
map_end = ((ram_end - 1) // GIB + 1) * GIB     # last block holding DRAM, exclusive end
if map_end != 0x8000_0000:
    fail(f"the virt Normal window is now {map_end:#x}; this test's arithmetic assumes 2 GiB")
# ...and it must really be the ceiling the allocator applies, not a duplicate constant.
if "crate::paging::NORMAL_MAP_END" not in open("kernel/src/frame.rs", encoding="utf-8").read():
    fail("frame.rs does not read paging::NORMAL_MAP_END — the ceiling is copied, not shared")

# --- part 1: the WIRING (the actual point of the milestone) -----------------------
if "board::resolved().ram_end()" not in main:
    fail("main.rs does not size the frame allocator from board::resolved() — the "
         "largest consumer on the board still bypasses rules 1-7")
if "ram_end_or" in main:
    fail("main.rs still reads fdt::ram_end_or (the raw, ruleless DRAM read)")
if "pub fn ram_end_or" in fdt:
    fail("fdt::ram_end_or still exists — a shortcut around board::resolve_from that "
         "any future consumer can reach for")

# --- part 2: what the live allocator reports --------------------------------------
need("frame allocator: bitmap over resolved DRAM")
m = re.search(r"^  resolved DRAM  (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \((\d+) MiB, (.+)\)",
              s, re.M)
if not m:
    fail("`frame` prints no resolved-DRAM row")
if int(m.group(1), 16) != ram_base or int(m.group(2), 16) != ram_end:
    fail(f"`frame` resolved DRAM is {m.group(1)}..{m.group(2)}, blueprint says "
         f"{ram_base:#010x}..{ram_end:#010x}")
if int(m.group(3)) != ram_mib:
    fail(f"`frame` resolved DRAM size is {m.group(3)} MiB, blueprint says {ram_mib}")
# QEMU's -kernel ELF path passes x0=0, so rule 1 keeps this row on the compile-time
# map. If it ever says otherwise here, a fixture has sized our memory.
if m.group(4) != "map (compile-time)":
    fail(f"resolved DRAM sourced '{m.group(4)}' on a boot with no live tree")

m = re.search(r"^  managed        (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \((\d+) MiB, (\d+) frames\)",
              s, re.M)
if not m:
    fail("`frame` prints no managed-range row")
base, end, managed_mib, frames = (int(m.group(1), 16), int(m.group(2), 16),
                                  int(m.group(3)), int(m.group(4)))
if base % FRAME_SIZE:
    fail(f"the first managed frame is not frame-aligned: {base:#x}")
if base <= ram_base:
    fail(f"management starts at {base:#x}, at or below DRAM base {ram_base:#x} — the "
         f"kernel image would be handed out as a free frame")
if end != base + frames * FRAME_SIZE:
    fail(f"managed range {base:#x}..{end:#x} does not hold {frames} frames")
if end > map_end:
    fail(f"managed range ends at {end:#x}, past the identity map at {map_end:#x} — a "
         f"frame handed out from there is a translation fault, not slow memory")
if end > ram_end:
    fail(f"managed range ends at {end:#x}, past the resolved DRAM top {ram_end:#x}")
# On this machine both bounds accept the range, so the whole of it (above the kernel)
# must be under management and the listing must say so rather than stay quiet.
if end != ram_end:
    fail(f"managed range stops at {end:#x} short of the resolved top {ram_end:#x}, yet "
         f"neither bound applies at 256 MiB")
need("  every frame of the resolved range is under management")
if "! BOUNDED" in s:
    fail("a bound was reported on a 256 MiB boot, which fits inside both limits")
if "the tree's /memory claim was refused" in s:
    fail("reported a rule-7 refusal on a boot with no live tree to refuse")
need("  below the base  the kernel image, its stacks and page tables (never handed out)")

# 2.732: both ceilings are stated on every boot, bound or not — a limit nobody can
# read before it fires is the thing 2.731 was about. The bitmap's figure is per-board
# now, so it can no longer be read off the bound message and must be printed here.
m = re.search(r"^  bitmap holds   (\d+) frames \((\d+) MiB\)", s, re.M)
if not m:
    fail("`frame` does not state the bitmap's capacity, so the bound is unreadable")
bm_frames, bm_mib = int(m.group(1)), int(m.group(2))
if bm_frames * FRAME_SIZE != bm_mib * (1 << 20):
    fail(f"bitmap states {bm_frames} frames but {bm_mib} MiB; they disagree")
# The whole of 2.732: the capacity is this board's declared DRAM clamped by the map,
# NOT the 256 MiB constant that happened to match virt and would have bound rpi4/pine64.
window_mib = (map_end - ram_base) >> 20
if bm_mib != min(ram_mib, window_mib):
    fail(f"bitmap holds {bm_mib} MiB, but this board declares {ram_mib} MiB inside a "
         f"{window_mib} MiB Normal window — expected min({ram_mib}, {window_mib})")
m = re.search(r"^  identity map   ends at (0x[0-9a-f]+)", s, re.M)
if not m:
    fail("`frame` does not state where the identity map ends")
if int(m.group(1), 16) != map_end:
    fail(f"`frame` says the identity map ends at {m.group(1)}, paging.rs says {map_end:#x}")

# --- part 3: `frame` describes the LIVE allocator, not a second opinion -----------
m = re.search(r"^frames: (\d+) total", s, re.M)
if not m or int(m.group(1)) != frames:
    fail(f"`mem` reports {m.group(1) if m else '?'} frames, `frame` reports {frames}")
m = re.search(r"^  phys frames    (\d+) x 4KiB", s, re.M)
if not m or int(m.group(1)) != frames:
    fail(f"`pmap` reports {m.group(1) if m else '?'} frames, `frame` reports {frames}")
# The count must also be arithmetic on the resolved range, not a stored guess.
if frames != (ram_end - base) // FRAME_SIZE:
    fail(f"{frames} frames is not (resolved top - base) / 4096 = "
         f"{(ram_end - base) // FRAME_SIZE}")

# --- part 4: the bounds, against synthetic ranges ---------------------------------
# `pmap test` (2.733) prints its own [ok] lines after these; split them apart so each
# block is judged on its own cases rather than on the other's.
cases, _, wcases = s.partition("identity map: Normal-window self-test over every known board")
cases = cases.split("bound self-test against synthetic ranges")[-1]
for phrase in ("a range inside both bounds is managed whole, and says so",
               "an unaligned start rounds UP, so no frame overlaps the kernel image",
               "an inverted range yields zero frames, not a wrapped count",
               # Bound 1 — the address space. The reason a bound exists at all.
               "DRAM past the identity map is bounded, and the bound is NAMED",
               "a bound reports the frames it cost, not just that it applied",
               # Bound 2 — this file's static array. A different kind of limit, and
               # the operator is owed the difference.
               "DRAM past the static bitmap is bounded, and the bound is a DIFFERENT reason",
               "a range of exactly the bitmap's capacity is NOT reported as bounded",
               "when both bounds bind, the TIGHTER one is the reason reported",
               "a range entirely past the identity map manages nothing (and says why)",
               "no plan ever exceeds the identity map or the bitmap, whatever it is offered",
               # 2.732 added two cases run against the REAL board map, not synthetic
               # ranges; this script keeps asserting them so 2.731's suite fails if the
               # board-derived bitmap is ever reverted to a constant.
               "this board's OWN declared DRAM is never bitmap-bounded",
               "the bitmap reserves bits for no frame the map cannot translate"):
    if f"[ok]   {phrase}" not in cases:
        fail(f"bound not proved: {phrase}")
if "[FAIL]" in cases:
    fail("a bound did not hold")
need("frame allocator: 12/12 bounds held")

# --- part 4b: where the ceiling COMES FROM (2.733) --------------------------------
# 2.731 made the ceiling shared rather than copied; 2.733 makes it *derived* rather
# than fixed. `pmap` must therefore state the window with its cause attached — the
# board's own declared DRAM — and the figure must be the same ceiling `frame` reports,
# not a second opinion printed next to it.
m = re.search(r"^  identity map   Normal (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \((\d+) x 1GiB "
              r"blocks?, from board DRAM (\d+) MiB @ (0x[0-9a-f]+)\)", s, re.M)
if not m:
    fail("`pmap` does not state the Normal window and where it came from")
win_start, win_end, win_blocks = int(m.group(1), 16), int(m.group(2), 16), int(m.group(3))
if win_end != map_end:
    fail(f"`pmap` says the Normal window ends at {win_end:#x}, `frame` says {map_end:#x}")
if win_start > ram_base or win_end < ram_end:
    fail(f"the Normal window {win_start:#x}..{win_end:#x} does not cover the resolved "
         f"DRAM {ram_base:#x}..{ram_end:#x} — mapped memory the allocator cannot use, "
         f"or usable memory the MMU cannot translate")
if win_blocks != (win_end - win_start) // GIB:
    fail(f"`pmap` claims {win_blocks} blocks over {win_end - win_start:#x} bytes")
if (int(m.group(4)), int(m.group(5), 16)) != (ram_mib, ram_base):
    fail(f"`pmap` attributes the window to {m.group(4)} MiB @ {m.group(5)}, the "
         f"blueprint declares {ram_mib} MiB @ {ram_base:#010x}")

# `pmap test` puts the derivation through every board in the table. This machine boots
# with one of them, so "the window follows each board's declared DRAM — and 2.732's
# clamp therefore goes slack on all of them" is otherwise a claim about code that has
# never run. The per-board expectations come from the SAME blueprint row as virt's.
if not wcases:
    fail("`pmap test` printed no Normal-window self-test")
cells = re.search(r"^\|\s*Main DRAM\s*\|(.+)$", doc, re.M).group(1).split("|")
for col, board in ((1, "rpi4"), (2, "pine64")):        # 0 is virt, parsed above
    mb = re.search(r"(\d+)\s*GiB", cells[col] if col < len(cells) else "")
    if not mb:
        fail(f"no DRAM size in the {board} column of the blueprint 'Main DRAM' row")
    declared = int(mb.group(1)) * 1024
    # The pattern runs to the END of the line the kernel prints, em dash included. It
    # used to stop at the block count because the capture was decoded latin1 and the
    # em dash arrived as mojibake; with the capture read as UTF-8 the tail is assertable,
    # and asserting it is not decoration — "— clamp slack" is the [ok] branch's own claim
    # that 2.732's clamp went slack on this board (paging.rs:376), which is the property
    # this loop exists to check and which no other line in the capture states.
    mw = re.search(rf"\[ok\]   board {board}: (\d+) MiB declared, window "
                   rf"(0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \((\d+) x 1GiB\) — clamp slack", wcases)
    if not mw:
        fail(f"`pmap test` does not clear board {board}: its declared DRAM is not "
             f"covered by the window built from it")
    if int(mw.group(1)) != declared:
        fail(f"board {board} declares {mw.group(1)} MiB in board.rs, {declared} MiB in "
             f"the blueprint")
    if (int(mw.group(3), 16) - int(mw.group(2), 16)) >> 20 < declared:
        fail(f"board {board}'s window is smaller than the {declared} MiB it declares — "
             f"2.732's clamp still binds there")
for phrase in ("2 GiB at 1 GiB spans TWO blocks, so a declared gibibyte is not clamped away",
               "a range straddling a 1 GiB boundary gets both blocks, not just the first",
               "exactly one gibibyte from zero is one block, not two",
               "a map declaring no DRAM gets no Normal blocks",
               "a map claiming DRAM past the address space is clamped to the table",
               "a DRAM base past the address space maps nothing at all",
               # The live map must BE that arithmetic, not sit beside it agreeing.
               "this board's ceiling IS window(board::ACTIVE), not a constant beside it",
               "mapping reaches past the Normal window far enough to include this board's UART"):
    if f"[ok]   {phrase}" not in wcases:
        fail(f"Normal window not proved: {phrase}")
if "[FAIL]" in wcases:
    fail("a Normal-window check did not hold")
mw = re.search(r"identity map: (\d+)/(\d+) window checks held \(Normal ends at "
               r"(0x[0-9a-f]+), (\d+) block", wcases)
if not mw:
    fail("`pmap test` prints no window-check tally")
if mw.group(1) != mw.group(2):
    fail(f"only {mw.group(1)} of {mw.group(2)} window checks held")
if int(mw.group(3), 16) != map_end or int(mw.group(4)) != win_blocks:
    fail(f"`pmap test` ends the window at {mw.group(3)} in {mw.group(4)} block(s); "
         f"`pmap` says {map_end:#x} in {win_blocks}")
wchecks = int(mw.group(2))

# --- part 5: the boot line agrees with the shell ----------------------------------
# Read from Part B's own boot, so this is the same kernel making the same claim twice
# to two different audiences — the operator at the prompt and the log nobody watches.
m = re.search(r"^\[mansa\] frame allocator: (\d+) frames \((\d+) MiB\) from (0x[0-9a-f]+) "
              # greedy, so the source's own parens ("map (compile-time)") survive
              r"\(RAM top (0x[0-9a-f]+), (.+)\)", boot, re.M)
if not m:
    fail("the boot line does not report the RAM top and its source")
if int(m.group(1)) != frames:
    fail(f"boot reported {m.group(1)} frames, the shell reports {frames}")
if int(m.group(3), 16) != base:
    fail(f"boot started the allocator at {m.group(3)}, the shell says {base:#010x}")
if int(m.group(4), 16) != ram_end:
    fail(f"boot sized the allocator to {m.group(4)}, resolved top is {ram_end:#010x}")
if m.group(5) != "map (compile-time)":
    fail(f"boot sourced the RAM top '{m.group(5)}' with no live tree present")
if "! bounded" in boot.split("[mansa] frame allocator")[-1][:400]:
    fail("boot claimed a bound that `frame` does not report")

print(f"FRAME-BOUNDS TEST PASSED  (allocator sized from board::resolved(), not from a "
      f"raw tree read — fdt::ram_end_or is gone; resolved DRAM {ram_base:#010x}.."
      f"{ram_end:#010x} ({ram_mib} MiB) per docs/15, sourced 'map (compile-time)' "
      f"because no live tree may size us; {frames} frames managed from {base:#010x} to "
      f"the resolved top, agreeing with mem and pmap, aligned, above the kernel image "
      f"and inside paging.rs's own {map_end:#x} Normal window; 12/12 bounds held "
      f"against synthetic ranges, each naming WHICH limit bound it and what it cost; "
      f"bitmap sized from board::ACTIVE at {bm_frames} frames ({bm_mib} MiB) = "
      f"min(declared {ram_mib} MiB, {window_mib} MiB map window), stated on every boot "
      f"rather than only when it binds; the {win_blocks}-block Normal window itself "
      f"derived from the board's declared DRAM, {wchecks}/{wchecks} window checks held "
      f"over every board in the table)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "FRAME-BOUNDS TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
