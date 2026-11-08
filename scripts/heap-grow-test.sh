#!/usr/bin/env bash
# Verify `heap grow` (milestone 2.740) — the arena stops being a link-time constant.
#
# 2.739 made the arena RECLAIM, which retires "it only ever fills" and leaves the older,
# larger objection untouched: the reservation is 1 MiB of `.bss` while `frame.rs` manages
# 222 MiB next door, so 2.735's honest answer to "how many more threads" was "about sixty",
# and reclaiming only meant the sixty could be REUSED. A workload legitimately holding 2 MiB
# live still died at a bound chosen at link time.
#
# `heap test` proves the growth arithmetic and drives one growth from inside the kernel.
# This suite makes the same claim from the SHELL, where an operator watches it, and against
# the figures the OTHER allocator reports — which is the check that matters, because the one
# way to fake growth is to count a frame as heap arena and as free memory at the same time.
# Checked here:
#   * a request LARGER THAN THE WHOLE RESERVATION is served — growth by elimination, since
#     no state of a fixed 1 MiB arena serves 1.5 MiB — and the address it is served at lies
#     OUTSIDE the reservation and inside the frame allocator's managed range;
#   * the block is written end to end and read back, so an unmapped or overlapping span is a
#     fault in this test rather than at whichever `format!` inherits it later;
#   * the bump cursor does NOT move: it walks the reservation and only ever the reservation,
#     so grown bytes reach the kernel through the free list alone;
#   * the frames come out of `frame::`: free frames fall by exactly what the heap says it
#     took, and `mem` reports the same figure from the frame allocator's own bitmap;
#   * growth is ONE-WAY — dropping the Vec returns the bytes to the list and keeps the
#     frames in the arena, so a second ask of the same size needs NO further growth (the
#     check that the list, not a second allocator, is where grown memory lives);
#   * `used + free == arena` still holds with a variable on the right, and `heap`, `mem`
#     and `sysinfo` all split that arena the SAME way rather than three opinions (`pmap`'s
#     two rows are checked by `heap-test.sh`, which runs it on the same boot as `mem`).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "HEAP GROW TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4876,server,nowait \
    -qmp tcp:127.0.0.1:4877,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/heapgrow.out"
# Two grows of the same width on purpose: the second must find the run already in the list.
python3 scripts/shell-demo.py 4876 4877 "${TMPDIR:-/tmp}/heapgrow.png" \
    "echo warmup" "heap grow" "heap grow" "heap" "mem" "sysinfo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "heap grow|grown|can grow|heap: |frames: |heap     :|arena " "$OUT" | sed 's/^/  /'

python3 - "$OUT" kernel/src/heap.rs <<'PY'
import re, sys
s    = open(sys.argv[1], "rb").read().decode("latin1")
heap = open(sys.argv[2], encoding="utf-8").read()

def fail(msg):
    print(f"HEAP GROW TEST FAILED ({msg})"); sys.exit(1)

def dash(t):
    """2.736's helper, adopted here for the reason its siblings adopted it: a pattern must
    not depend on which encoding the capture happened to arrive in. Matching the console's
    em dash with a single `.` assumes ONE character — true only while `shell-demo.py`
    re-encoded its stdout to cp1252. Needles are code points, not typed characters."""
    for needle in (chr(0x2014), chr(0x97), chr(0xE2) + chr(0x80) + chr(0x94)):
        t = t.replace(needle, "-")
    return t

FRAME = 4096

# --- part 0: growth is reached from the REFUSAL path, in the source -------------------
# The property that keeps growth honest and cannot be read off the console: while the bump
# can serve a request, the arena is a `.bss` reservation and costs the frame allocator
# nothing. A `grow()` called anywhere else would be a heap that takes frames it does not
# need, on a machine whose other allocator is the one under pressure.
alloc_impl = heap.split("unsafe impl GlobalAlloc for KernelAlloc {")[-1].split("\n}")[0]
if alloc_impl.count("grow(") != 1:
    fail(f"`alloc` reaches growth from {alloc_impl.count('grow(')} places, not 1 — growth "
         f"belongs on the refusal path only, or the heap takes frames it did not need")
if "Err(why) =>" not in alloc_impl or "grow(layout.size(), layout.align())" not in alloc_impl:
    fail("growth is no longer reached from the refusal path — the bump is supposed to be "
         "tried FIRST and growth to be what happens when it says no")
if "GROW_LOCK" not in heap or "compare_exchange" not in heap:
    fail("growth takes no lock — `frame::alloc` walks a static bitmap with no lock of its "
         "own, and growth is the first caller that runs at arbitrary times")

# --- part 1: the ask, and what it means -----------------------------------------------
asks = [m for m in re.finditer(
    r"\[heap grow\] asking for (\d+) B \((\d+) KiB\).{1,5} the reservation is (\d+) B, "
    r"the arena is (\d+) B", s)]
if len(asks) != 2:
    fail(f"{len(asks)} `heap grow` asks in the capture, expected 2")
want, reserved = int(asks[0].group(1)), int(asks[0].group(3))
if want <= reserved:
    fail(f"the demo asks for {want} B into a {reserved} B reservation — a request a FIXED "
         f"arena could serve proves nothing about growth")
if f"[heap grow] that is LARGER than the whole .bss reservation" not in s:
    fail("`heap grow` does not say that its ask exceeds the reservation, which is the "
         "whole reason a served request is growth by elimination")
arena_before = int(asks[0].group(4))
if arena_before != reserved:
    fail(f"the arena is already {arena_before} B before the first ask, not the {reserved} B "
         f"reserved — something grew it before the test did")

# --- part 2: served, out of a frame, and read back ------------------------------------
served = [m for m in re.finditer(
    r"\[heap grow\] served (\d+) B at (0x[0-9a-f]+).{1,5} (OUTSIDE|inside) the reservation "
    r"\(read back (OK|MISMATCH) of (\d+) B\)", s)]
if len(served) != 2:
    fail(f"{len(served)} served lines, expected 2 — a `heap grow` did not complete")
for i, m in enumerate(served):
    if int(m.group(1)) != want:
        fail(f"ask {i}: served {m.group(1)} B, asked {want} B")
    if m.group(3) != "OUTSIDE":
        fail(f"ask {i}: a {want} B block was served INSIDE the {reserved} B reservation — "
             f"arithmetically impossible unless the figure is not the live arena")
    if m.group(4) != "OK":
        fail(f"ask {i}: the block did not read back — the grown span is not the memory the "
             f"heap handed out")

# --- part 3: the arena grew, and by exactly what it took ------------------------------
m = re.search(r"\[heap grow\] arena (\d+) B -> (\d+) B \((\d+) B grown in (\d+) frame\(s\) "
              r"over (\d+) growth\(s\)\); bump (\d+) B -> (\d+) B", s)
if not m:
    fail("`heap grow` does not report the arena before and after")
a0, a1, grown, gframes, grows = (int(m.group(i)) for i in range(1, 6))
bump0, bump1 = int(m.group(6)), int(m.group(7))
if a1 <= a0:
    fail(f"the arena did not grow: {a0} B -> {a1} B, yet a {want} B request was served")
if a1 - a0 != grown or grown != gframes * FRAME:
    fail(f"the arena moved {a1 - a0} B, the heap says it grew {grown} B in {gframes} "
         f"frame(s) ({gframes * FRAME} B) — three figures for one growth")
if grown < want:
    fail(f"{grown} B grown for a {want} B request — the arena grew less than it served")
if grows < 1:
    fail("the arena is larger and the heap recorded no growth event")
# The bump walks the reservation and ONLY the reservation. If growth had extended the bump
# instead of feeding the list, this is where it would show.
if bump1 != bump0:
    fail(f"the bump cursor moved {bump0} B -> {bump1} B while serving a request out of "
         f"GROWN memory — grown bytes are supposed to reach the kernel through the list")
if bump1 > reserved:
    fail(f"the bump cursor is at {bump1} B, past the {reserved} B reservation")

# --- part 4: the frames came from the OTHER allocator ---------------------------------
frames = [m for m in re.finditer(
    r"\[heap grow\] frames (\d+) -> (\d+) free \((\d+) taken by the heap", s)]
if len(frames) != 2:
    fail("`heap grow` does not report the frame pool on both sides of the ask")
f0, f1, took = (int(frames[0].group(i)) for i in range(1, 4))
if f0 - f1 != took:
    fail(f"free frames fell {f0 - f1} while the heap says it took {took} — a frame counted "
         f"as heap arena AND as free memory reads as the machine having more RAM than it has")
if took != gframes:
    fail(f"the heap took {took} frame(s) from the pool but grew by {gframes} frame(s)")
if took == 0:
    fail("no frame left the frame allocator, yet the arena grew — the growth is bookkeeping")

# --- part 5: growth is ONE-WAY, and the second ask proves where grown memory lives -----
frees = [m for m in re.finditer(
    r"\[heap grow\] freed .{1,5} arena STAYS (\d+) B \((\d+) growth\(s\) this ask\), "
    r"(\d+) B in the list, used (\d+) B", s)]
if len(frees) != 2:
    fail("`heap grow` does not report the arena after the block is dropped")
stay, this_ask, in_list, used_after = (int(frees[0].group(i)) for i in range(1, 5))
if stay != a1:
    fail(f"the arena is {stay} B after the drop and was {a1} B before it — growth is "
         f"supposed to be one-way: the frames stay the arena's")
if in_list < want:
    fail(f"only {in_list} B went back to the list after a {want} B block was dropped")
# THE claim of the second ask: the same width again, and the arena must not grow, because
# the run is already in the list. This is what separates "the list holds grown memory" from
# "a second allocator hands out frames".
second_ask_growths = int(frees[1].group(2))
if second_ask_growths != 0:
    fail(f"a second identical ask grew the arena {second_ask_growths} more time(s) — the "
         f"1.5 MiB freed by the first ask is not being served out of the list")
if int(frees[1].group(1)) != stay:
    fail(f"the arena is {frees[1].group(1)} B after the second ask and {stay} B after the "
         f"first, with no growth between them")
if int(frames[1].group(3)) != 0:
    fail(f"the second ask took {frames[1].group(3)} more frame(s) for memory the list "
         f"already held")

# --- part 6: it still adds up, with a variable on the right ---------------------------
adds = [m for m in re.finditer(
    r"\[heap grow\] adds up: used (\d+) \+ free (\d+) == arena (\d+) \((yes|NO)\), "
    r"bump (\d+) <= reservation (\d+) \((yes|NO)\)", s)]
if len(adds) != 2:
    fail("`heap grow` does not restate the arena identity after the ask")
for i, m in enumerate(adds):
    if m.group(4) != "yes":
        fail(f"ask {i}: used {m.group(1)} + free {m.group(2)} != arena {m.group(3)} — the "
             f"identity 2.734 held over a constant does not hold over the variable")
    if m.group(7) != "yes":
        fail(f"ask {i}: the bump cursor {m.group(5)} B is past the {m.group(6)} B reservation")
    if int(m.group(3)) != stay:
        fail(f"ask {i}: the identity is stated over a {m.group(3)} B arena, the report says "
             f"{stay} B")

# --- part 7: one arena, described four times ------------------------------------------
m = re.search(r"^  grown          (\d+) B in (\d+) frame\(s\) over (\d+) growth\(s\)", s, re.M)
if not m:
    fail("`heap` does not state the grown part of the arena")
if (int(m.group(1)), int(m.group(2))) != (stay - reserved, gframes):
    fail(f"`heap` says {m.group(1)} B grown in {m.group(2)} frame(s); the growth itself "
         f"reported {stay - reserved} B in {gframes}")
if int(m.group(3)) < 1:
    fail("`heap` reports no growth event after two `heap grow` asks")
# 2.741 reworded this row: the figure OFFERED is the heap's share, and the pool it was
# quoting before is printed beside it. Both are still checked — the pool against what the
# growth above left, and the offer against the pool it must now be smaller than. (The dash
# is FOLDED, not matched as `.`: one character or three depending on how the capture was
# written out, which is the harness's business and must not be this pattern's.)
m = re.search(r"^  can grow       (\d+) more frame\(s\) \(\d+ MiB\) - its SHARE, not the "
              r"pool: (\d+) free next door", dash(s), re.M)
if not m:
    fail("`heap` does not state that the arena can still grow — the ceiling being the "
         "MACHINE's rather than the linker's is 2.740's whole claim")
if int(m.group(2)) != f1:
    fail(f"`heap` sees {m.group(2)} free frame(s) next door, the growth left {f1} — the "
         f"heap and the frame allocator disagree about the pool they share")
if int(m.group(1)) >= f1:
    fail(f"`heap` offers {m.group(1)} of the {f1} frame(s) free: the 2.741 share and floor "
         f"are not being subtracted, so the row still promises the whole pool")

m = re.search(r"^heap:   (\d+) / (\d+) bytes used \(\d+ B high-water, arena (\d+) KiB @ "
              r"(0x[0-9a-f]+) = (\d+) KiB reserved \+ (\d+) KiB grown\)", s, re.M)
if not m:
    fail("`mem` does not report the grown arena")
if int(m.group(2)) != stay:
    fail(f"`mem` reports a {m.group(2)} B arena, `heap grow` a {stay} B one")
if int(m.group(5)) * 1024 != reserved or int(m.group(6)) * 1024 != stay - reserved:
    fail(f"`mem` splits the arena as {m.group(5)} KiB reserved + {m.group(6)} KiB grown, "
         f"which is not {reserved} B + {stay - reserved} B")
m = re.search(r"^frames: (\d+) total \(\d+ MiB of 4KiB physical frames\), (\d+) used.{1,5} "
              r"(\d+) of them the heap's arena", s, re.M)
if not m:
    fail("`mem` does not report used frames and how many are the heap's — the figure that "
         "makes 'the heap took N frames' checkable against the allocator they came from")
tot_f, used_f, heap_f = (int(m.group(i)) for i in range(1, 4))
if heap_f != gframes:
    fail(f"`mem` says the heap holds {heap_f} frame(s), the growth took {gframes}")
if heap_f > used_f or used_f > tot_f:
    fail(f"{heap_f} heap frames of {used_f} used of {tot_f} total — the books do not nest")
if tot_f - used_f != f1:
    fail(f"`mem` leaves {tot_f - used_f} frame(s) free, `heap grow` reported {f1} — the two "
         f"allocators are counting the same pool differently")
m = re.search(r"^  heap     : (\d+) / (\d+) bytes used \((\d+) B reserved \+ (\d+) B grown "
              r"from frames\)", s, re.M)
if not m:
    fail("`sysinfo` does not report the grown arena")
if (int(m.group(2)), int(m.group(3)), int(m.group(4))) != (stay, reserved, stay - reserved):
    fail(f"`sysinfo` describes a different arena: {m.group(2)} B = {m.group(3)} + {m.group(4)}")

if "! cannot grow" in s:
    fail("the heap reports it cannot grow on a board with 222 MiB of managed frames")
if "[heap] REFUSED" in s:
    fail("a request was refused during a growth run that had frames available")

print(f"HEAP GROW TEST PASSED  ({want} B asked of a {reserved} B .bss reservation and "
      f"SERVED — outside the reservation, read back end to end, at an address inside the "
      f"frame allocator's managed range; the arena grew {stay - reserved} B in {gframes} "
      f"frame(s) and the frame pool fell by exactly those {took}, so neither allocator is "
      f"counting the other's memory; the bump cursor did not move one byte ({bump0} B, "
      f"under the {reserved} B reservation), so grown memory reaches the kernel through the "
      f"free list alone; dropping the block returned {in_list} B to the list and left the "
      f"arena at {stay} B, and a SECOND identical ask was served with 0 further growths and "
      f"0 further frames — the list, not a second allocator, is where grown memory lives; "
      f"used + free == arena still holds with a variable on the right, and heap / mem / "
      f"sysinfo split that arena the same way rather than three opinions; the heap "
      f"and the frame allocator agree {f1} frame(s) remain, so the ceiling is the machine's "
      f"now and not the linker's)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "HEAP GROW TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
