#!/usr/bin/env bash
# Verify `heap` / `heap test` (milestone 2.734) — the THIRD memory ceiling.
#
# 2.731 made the frame allocator's bounds visible before they fire and named them when
# they do; 2.732 sized the bitmap from the board and 2.733 the identity map. The kernel
# heap was the constant left over: a fixed 1 MiB `.bss` arena, identical on a 256 MiB
# virt and a 2 GiB pine64, whose only failure signal was a null pointer from `alloc()`
# that surfaced at whichever caller happened to `unwrap` it — with nothing attached
# saying the heap was what ran out.
#
# Placement stays compile-time (the arena lives in the image; it is reserved before
# there is a tree to parse). What must not stay invisible is the BOUND. Checked here:
#   * `heap` states the arena's range, its use, its high-water mark and the padding —
#     the ceiling readable before it bites, not only after;
#   * the arena lies BELOW the frame allocator's managed base, so no frame can ever be
#     handed out on top of it. Cross-checked against what `frame` reports, i.e. against
#     the other allocator, not against a number heap.rs states about itself;
#   * `mem` and `pmap` report the SAME arena as `heap` — one heap described three times,
#     not three opinions;
#   * exhaustion is a NAMED refusal in the source: `alloc` routes through the pure
#     `place()` and prints the bound before returning null;
# 2.740 moved the ceiling itself: the arena is the `.bss` reservation PLUS every frame the
# heap has since taken from `frame::alloc()`, so every total on this screen is a sum over a
# variable. What is checked here is that all three reports (`heap`, `mem`, `pmap`) describe
# ONE arena split the same way — same reservation, same grown part — and that the frames the
# heap claims are frames the frame allocator agrees it handed out, which is the double-count
# the milestone is ordered to avoid.
#   * and the refusal paths themselves are exercised by `heap test` against SYNTHETIC
#     requests. The live kernel allocates a few KiB into a mebibyte, so the exact-fit
#     boundary, the alignment that exhausts what the size alone would fit, the
#     larger-than-the-arena request and the race two cores lose are otherwise all claims
#     about code that has never run.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "HEAP TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4866,server,nowait \
    -qmp tcp:127.0.0.1:4867,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/heap.out"
python3 scripts/shell-demo.py 4866 4867 "${TMPDIR:-/tmp}/heap.png" \
    "echo warmup" "heap" "heap test" "mem" "pmap" "frame" \
    "heap grow 65536" "heap" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "kernel heap|arena |used  |high-water|below frames|on exhaustion|heap arena|heap: |\[ok\]|\[FAIL\]|bump checks held|managed |\[heap grow\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" kernel/src/heap.rs kernel/src/shell.rs <<'PY'
import re, sys
s     = open(sys.argv[1], "rb").read().decode("latin1")
heap  = open(sys.argv[2], encoding="utf-8").read()
shell = open(sys.argv[3], encoding="utf-8").read()

def fail(msg):
    print(f"HEAP TEST FAILED ({msg})"); sys.exit(1)

def dash(t):
    """2.736's helper, adopted here for the reason its siblings adopted it: a pattern
    must not depend on which encoding the capture happened to arrive in. This suite used
    to match the console's em dash with a single `.`, which quietly assumed ONE character
    — true only while `shell-demo.py` re-encoded its stdout to cp1252. Folding every form
    to one ASCII marker holds whichever way the capture is written, and the needles are
    built from code points rather than typed so re-saving this file cannot break one."""
    for needle in (chr(0x2014), chr(0x97), chr(0xE2) + chr(0x80) + chr(0x94)):
        t = t.replace(needle, "-")
    return t

def need(*frags):
    for f in frags:
        if f not in s:
            fail(f"missing '{f}'")

# --- part 1: exhaustion is NAMED, in the source ------------------------------------
# The behaviour this asserts cannot be observed from the shell: the live kernel never
# comes near 1 MiB, so the refusal path never executes on a real boot. What CAN be held
# is that the path exists and that nothing bypasses it.
if not re.search(r"place\(base,\s*cur,\s*HEAP_SIZE,\s*layout\.size\(\),\s*layout\.align\(\)\)", heap):
    fail("alloc() does not route through the pure place() — the bump arithmetic is back "
         "to being inline, where no self-test can reach it")
# Exactly one refusal site, and it is the named one. Scoped to the ALLOCATOR since 2.738:
# `null_mut()` now has a second, unrelated meaning in this file — the free list's head
# terminator and its scan cursor, where null means "end of list", not "refused". What
# 2.734 removed was an unattributed *allocation failure*, so that is what is counted here;
# every other null in the file must belong to the list, which the second check pins. A
# guard that could not tell the two apart would have forced the list to be written
# elsewhere, or this claim to be dropped.
# 2.739 renamed the type `BumpAlloc` -> `KernelAlloc`, because the bump stopped being the
# whole of it. Split on a name the file must actually contain, and say so rather than
# silently scoping to nothing: a missing separator makes `split` return the WHOLE source,
# whose first "\n}" is hundreds of lines above `alloc`, so the count below would come out 0
# and this check would pass by finding no allocator at all.
if "unsafe impl GlobalAlloc for KernelAlloc {" not in heap:
    fail("no `impl GlobalAlloc for KernelAlloc` — the allocator was renamed or moved, and "
         "every claim below would be scoped to the wrong text")
alloc_impl = heap.split("unsafe impl GlobalAlloc for KernelAlloc {")[-1].split("\n}")[0]
if alloc_impl.count("null_mut()") != 1:
    fail(f"the allocator returns null from {alloc_impl.count('null_mut()')} places, not 1 "
         f"— a second bare null is exactly the unattributed failure 2.734 removed")
free_list = heap.split("impl FreeList {")[-1].split("\n}")[0]
if heap.count("null_mut()") != 1 + free_list.count("null_mut()"):
    fail("heap.rs has a null_mut() that is neither the allocator's single named refusal "
         "nor the free list's own terminator — an unattributed null by either meaning")
if not re.search(r"record_fail\(why\);\s*\n\s*crate::kprintln!\(\s*\n\s*\"\[heap\] REFUSED", heap):
    fail("the refusal is not recorded AND named on the console before returning null")
for const in ("BOUND_ARENA", "BOUND_REQUEST", "BOUND_ALIGN",
              # 2.740: growth failures are a DIFFERENT allocator's bound, and an operator
              # told "the arena is full" when the machine is out of frames goes hunting a
              # heap leak on a box that has no RAM left.
              "BOUND_FRAMES", "BOUND_UNMAPPED", "BOUND_GROW",
              # 2.741: and the two that are POLICY rather than accident — the machine would
              # have said yes and the kernel said no, which is a different thing to be told.
              "BOUND_CAP", "BOUND_RESERVE"):
    if f"pub const {const}: &str" not in heap:
        fail(f"heap::{const} is gone — the bound would be a message built at the failure "
             f"site again, which no test can assert on")
if 'crate::heap::selftest()' not in shell and 'heap::selftest()' not in shell:
    fail("shell.rs does not expose `heap test`")

# --- part 2: what the live arena reports -------------------------------------------
# 2.739: the arena's own headline changed, because what sits under it did. The bump is
# still there and still named — it is the fallback now, not the whole allocator.
need("kernel heap: free-list arena in .bss, with a bump behind it")
m = re.search(r"^  arena          (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \((\d+) KiB, compile-time",
              s, re.M)
if not m:
    fail("`heap` prints no arena row")
hbase, hend, hkib = int(m.group(1), 16), int(m.group(2), 16), int(m.group(3))
if hend - hbase != hkib * 1024:
    fail(f"arena {hbase:#x}..{hend:#x} is not the {hkib} KiB it claims")
if hbase % 16:
    fail(f"the arena base {hbase:#x} is not 16-byte aligned; every Layout align > 16 "
         f"would then pay padding the arithmetic did not predict")
# It lives in the image, which QEMU's -kernel loads at 0x40000000.
if not (0x4000_0000 <= hbase < 0x4800_0000):
    fail(f"the arena is at {hbase:#x}, outside the loaded kernel image — a `.bss` "
         f"reservation cannot be there")

# 2.740: the reservation is no longer the arena. `heap` states the grown part BEFORE it has
# fired (this row is printed with its zeros), because an operator who cannot see the figure
# cannot tell an arena that CAN grow from one that has merely not run out yet.
m = re.search(r"^  grown          (\d+) B in (\d+) frame\(s\) over (\d+) growth\(s\)", s, re.M)
if not m:
    fail("`heap` does not state the grown part of the arena — the 2.740 row is gone, and "
         "every total below would silently mean the reservation again")
grown, gframes, grows = int(m.group(1)), int(m.group(2)), int(m.group(3))
if gframes * 4096 != grown:
    fail(f"{grown} B grown is not the {gframes} frame(s) it claims")
if (grown == 0) != (grows == 0):
    fail(f"{grows} growth(s) produced {grown} B — a growth that grew nothing, or bytes "
         f"that arrived without one")
m = re.search(r"arena is (\d+) B, not the (\d+) B reserved", s)
if not m:
    fail("`heap` does not restate the arena against the reservation")
arena, reserved = int(m.group(1)), int(m.group(2))
if reserved != hkib * 1024 or arena != reserved + grown:
    fail(f"the arena row says {hkib} KiB reserved, the grown row says {reserved} B reserved "
         f"+ {grown} B grown = {arena} B — two different arenas on one screen")
# ...and whether it could grow AGAIN, asked of the OTHER allocator. Checked against what
# `frame` reports below, not against a number heap.rs states about itself.
# 2.741: the figure offered is the heap's SHARE, not the pool. The pool is still printed
# beside it, because the difference between the two is the whole point of the policy — an
# operator who can only see the share cannot tell a capped heap from a full machine.
# The dash is FOLDED on both sides rather than matched as `.`: the capture is read
# byte-preserving, so the console's em dash is one character or three depending on how it
# was written out, and a pattern that assumes either is a pattern that breaks when the
# harness is fixed. `dash()` makes the question not arise.
m = re.search(r"^  can grow       (\d+) more frame\(s\) \((\d+) MiB\) - its SHARE, not the "
              r"pool: (\d+) free next door, cap 1/(\d+) of managed, floor 1/(\d+) kept",
              dash(s), re.M)
if not m:
    if "! cannot grow" in s:
        fail("the heap reports it cannot grow at all on an ordinary boot, with 222 MiB "
             "of managed frames next door")
    fail("`heap` does not state whether the arena can still grow, and by how much its "
         "POLICY would let it (2.741)")
canfree, poolfree = int(m.group(1)), int(m.group(3))
share_den, floor_den = int(m.group(4)), int(m.group(5))
if canfree == 0:
    fail("the heap reports no frame free to grow into on a boot that uses a few MiB")
if share_den < 2 or floor_den < 2:
    fail(f"a cap of 1/{share_den} and a floor of 1/{floor_den}: a share that is the whole "
         f"machine is not a share, and growth would be unbounded again")
if canfree >= poolfree:
    fail(f"`heap` offers {canfree} frame(s) of the {poolfree} free next door — the row is "
         f"still quoting the POOL, so the cap and the floor are not being subtracted")

m = re.search(r"^  used           (\d+) B \((\d+) B of it alignment padding\), (\d+) B free",
              s, re.M)
if not m:
    fail("`heap` does not state its use")
used, pad, free = int(m.group(1)), int(m.group(2)), int(m.group(3))
# `arena` and not the reservation since 2.740 — `used + free` is a sum over a variable now.
if used + free != arena:
    fail(f"{used} B used + {free} B free != the {arena} B arena "
         f"({hkib} KiB reserved + {grown} B grown)")
if pad > used:
    fail(f"{pad} B of padding inside {used} B used")
if used == 0:
    fail("the arena reports nothing used, yet the kernel Vec-demos on it at boot — the "
         "figure is not coming from the live allocator")

m = re.search(r"^  high-water     (\d+) B", s, re.M)
if not m:
    fail("`heap` states no high-water mark")
high = int(m.group(1))
if high < used:
    fail(f"high-water {high} B is below the {used} B currently used")
# 2.734 asserted these two were EQUAL, and that was the honest claim then: `dealloc` was a
# no-op, so `used` only ever rose and the mark it set was itself. 2.739 inverts it. The gap
# is now the milestone — memory the machine finished with and got to use again — so the
# check that used to prove the figures were maintained becomes the check that they are
# maintained SEPARATELY. Equality here would mean `dealloc` never lowered `used` on a boot
# whose `heap test` above ran hundreds of live cycles.
m2 = re.search(r"^  high-water     \d+ B - used has since fallen by (\d+) B", dash(s), re.M)
if not m2:
    fail("`heap` no longer states the fall from the high-water mark, which is 2.739's "
         "entire visible claim")
fell = int(m2.group(1))
if fell != high - used:
    fail(f"`heap` says used fell {fell} B, but high-water {high} B - used {used} B is "
         f"{high - used} B — the row is arithmetic on two different snapshots")
if fell == 0:
    fail(f"high-water {high} B equals used {used} B, so `used` never fell on a boot that "
         f"freed — `dealloc` is back to being the no-op 2.739 replaced")

# --- part 3: the arena is below the frames, checked against the OTHER allocator -----
m = re.search(r"^  below frames   arena top (0x[0-9a-f]+) < managed base (0x[0-9a-f]+)", s, re.M)
if not m:
    fail("`heap` does not state that the arena lies below the managed base (or it does "
         "not — look for '! OVERLAP')")
if int(m.group(1), 16) != hend:
    fail(f"`heap` says the arena tops at {m.group(1)} in one row and {hend:#x} in another")
fbase = int(m.group(2), 16)
m = re.search(r"^  managed        (0x[0-9a-f]+)\.\.(0x[0-9a-f]+) \((\d+) MiB, (\d+) frames\)",
              s, re.M)
if not m:
    fail("`frame` prints no managed-range row to check the arena against")
total_frames = int(m.group(4))
# 2.740's other-allocator cross-check: the frames `heap` says it could still grow into have
# to be frames `frame` says exist. A heap quoting a pool bigger than the machine's is the
# double-count the whole milestone is ordered to avoid.
if canfree > total_frames:
    fail(f"`heap` offers {canfree} frame(s) to grow into, `frame` manages {total_frames} "
         f"in total — the heap is counting memory the machine does not have")
if int(m.group(1), 16) != fbase:
    fail(f"`heap` claims the frames start at {fbase:#x}, `frame` says {m.group(1)} — the "
         f"claim 'no frame overlaps the arena' is being made against a stale number")
if hend > fbase:
    fail(f"the arena tops at {hend:#x}, at or above the first managed frame {fbase:#x} — "
         f"the allocator can hand out a frame that IS the heap")
need("  on exhaustion  a NAMED refusal on the console, then a null")
if "! refused" in s:
    fail("the heap refused a request on an ordinary boot")
if "! OVERLAP" in s:
    fail("the arena overlaps the managed frames")

# --- part 4: one heap, described three times ---------------------------------------
# NOTE the ordering: `heap` ran BEFORE `heap test`, and `heap test` grows the arena (it
# serves a 1.5 MiB request out of frames). So `mem` and `pmap` here report an arena that is
# demonstrably LARGER than the one part 2 read — which is the point. What must hold is not
# that the three figures are equal but that all three describe the same arena: the same
# reservation at the same base, plus whatever has since been grown into it.
m = re.search(r"^heap:   (\d+) / (\d+) bytes used \((\d+) B high-water, arena (\d+) KiB @ "
              r"(0x[0-9a-f]+) = (\d+) KiB reserved \+ (\d+) KiB grown\)", s, re.M)
if not m:
    fail("`mem` does not report the arena (or still reports heap::size(), the reservation, "
         "as the total — a smaller arena that no longer exists)")
mtotal, mkib, mbase = int(m.group(2)), int(m.group(4)), int(m.group(5), 16)
mres, mgrown = int(m.group(6)), int(m.group(7))
if (mres, mbase) != (hkib, hbase):
    fail(f"`mem` reserves {mres} KiB @ {mbase:#x}, `heap` says {hkib} KiB @ {hbase:#x}")
if mkib != mres + mgrown or mtotal != mkib * 1024:
    fail(f"`mem` says arena {mkib} KiB but {mres} reserved + {mgrown} grown, total "
         f"{mtotal} B — the row is arithmetic on two different arenas")
if mgrown == 0:
    fail("`mem` reports nothing grown AFTER `heap test` served a 1.5 MiB request that no "
         "1 MiB reservation could ever serve — the growth is not reaching the totals")
if int(m.group(3)) < int(m.group(1)):
    fail("`mem` reports a high-water mark below its own used figure")
m = re.search(r"^  heap reserved  (0x[0-9a-f]+)\.\.(0x[0-9a-f]+)  (\d+) bytes of .bss "
              r"\(below the managed base\)", s, re.M)
if not m:
    fail("`pmap` does not report the reservation and where it sits")
if (int(m.group(1), 16), int(m.group(2), 16), int(m.group(3))) != (hbase, hend, hkib * 1024):
    fail(f"`pmap` describes a different reservation than `heap`: {m.group(1)}..{m.group(2)}")
# The grown part is not a range — it is however many frames the heap took out of the pool
# below — so `pmap` prints it as a second row rather than widening the first, which would
# claim a contiguous span that does not exist.
m = re.search(r"^  heap grown     (\d+) bytes in (\d+) frame\(s\) from the pool below", s, re.M)
if not m:
    fail("`pmap` does not report the grown part of the arena")
pgrown, pframes = int(m.group(1)), int(m.group(2))
if pgrown != pframes * 4096 or pgrown != mgrown * 1024:
    fail(f"`pmap` says {pgrown} B in {pframes} frame(s); `mem` says {mgrown} KiB grown — "
         f"two different arenas again")
m = re.search(r"arena is (\d+) / (\d+) bytes used", s)
if not m or int(m.group(2)) != mtotal:
    fail("`pmap`'s arena total disagrees with `mem`'s")
# And the frames it took left the OTHER allocator: `pmap` reports both sides on one row, so
# a frame counted as heap arena and as free memory at once would read as the machine having
# more RAM than it has.
m = re.search(r"^  phys frames    (\d+) x 4KiB = (\d+) MiB total, (\d+) used "
              r"\((\d+) of them the heap's\)", s, re.M)
if not m:
    fail("`pmap` does not state how many frames are used and how many of them are the heap's")
if int(m.group(1)) != total_frames:
    fail(f"`pmap` counts {m.group(1)} frames, `frame` manages {total_frames}")
if int(m.group(4)) != pframes:
    fail(f"`pmap` says the heap holds {m.group(4)} frame(s) in one row and {pframes} in "
         f"another")
if int(m.group(4)) > int(m.group(3)):
    fail(f"the heap holds {m.group(4)} frame(s) out of {m.group(3)} used in the whole "
         f"machine — frames counted by the heap that the frame allocator never handed out")

# --- part 5: the refusal paths, against synthetic requests --------------------------
cases = s.split("kernel heap: bump self-test against synthetic requests")[-1]
for phrase in (
        "an aligned request at an empty arena starts at the base and costs nothing extra",
        # Alignment padding — the reason `used` and "what callers asked for" differ.
        "an unaligned bump pads FORWARD to the alignment, and the padding is charged",
        "an offset already aligned pays no padding",
        # The exact-fit boundary, both sides of it.
        "a request that exactly fills the arena is SERVED, to the last byte",
        "one byte past the exact fit is refused, and NAMED as the arena having no room",
        "a request that fits but whose ALIGNMENT does not is refused, not truncated",
        # The second bound: not pressure on the heap, a caller asking the impossible.
        "a request larger than the WHOLE arena is a different bound, even on an empty one",
        "absurd sizes and a base at the top of the address space refuse, never wrap",
        "an alignment that is not a power of two is refused by name, not masked",
        "a zero-byte request is served and costs the arena nothing",
        # The race two cores would lose without the CAS retry.
        "two cores off the same USED compute the SAME bytes — the race is real",
        "re-placing after a lost CAS lands past the winner, so no byte is handed out twice",
        "over a run of mixed requests, used only ever grows and never leaves the arena",
        "the REAL arena is 16-byte aligned and lies below the first managed frame",
        # 2.740: the arithmetic growth asks BEFORE it takes a frame. `span_mapped`'s
        # refusing branch is the one that cannot be reached from a live board — this board's
        # managed range is already clamped to the identity map — and it is the branch whose
        # failure is a fault on the first node write rather than a red test.
        "a span is mapped only if it ENDS inside the map",
        "a span at the top of the address space says NOT mapped rather than wrapping into a false yes",
        "a request is rounded UP to whole frames, and the last byte over a frame costs a whole one",
        "the alignment pad is asked for as well, so a grown run can still be carved to alignment",
        "a width that cannot be counted in frames refuses rather than wrapping to a small one",
        "a growth bound names a DIFFERENT allocator than the arena bound, and says so distinctly",
        # 2.741: growth's share. The accidents above stop growth when the machine cannot
        # serve it; these stop it while the machine still can, which is the only bound that
        # keeps frames for the callers that cannot grow, retry or refuse.
        "an ask well inside both the share and the floor is ALLOWED, and costs no bound",
        "the last frame under the share is served and the one that crosses it is refused BY POLICY",
        "growth stops at the frame floor kept for page tables and drivers, and NAMES the floor",
        "the share is resolved against the pool as it is, so a bigger machine grants the same ask",
        "absurd arenas, asks and pools refuse by policy rather than overflowing into a yes",
        "a POLICY refusal is a different string from out-of-frames and from arena-full",
        "the offered headroom is the SMALLER of the share left and the frames above the floor",
        "an ask of exactly the offered headroom is served, and one frame more is refused",
        # ...and the growth itself, live: the claim no arithmetic above can make.
        "a 1.5 MiB request",
        "the memory it was served from is a FRAME, outside the reservation and inside the identity map",
        "the grown bytes come back to the LIST when freed, the arena keeps them, and the machine can still grow again"):
    # (the serial capture is decoded latin1, so an em dash in a case name is not a
    #  literal "—" here; compare on the part before it)
    if f"[ok]   {phrase.split(' —')[0]}" not in cases:
        fail(f"bound not proved: {phrase}")
if "[FAIL]" in cases:
    fail("a bump check did not hold")
# The tally splits the arena into its two parts since 2.740, for the reason `mem` does: the
# figure it used to print was `HEAP_SIZE`, which stopped being the arena the moment growth
# landed.
m = re.search(r"kernel heap: (\d+)/(\d+) bump checks held \(arena (\d+) KiB at "
              r"(0x[0-9a-f]+).{1,5} (\d+) KiB reserved \+ (\d+) KiB grown, (\d+) B used, "
              r"(\d+) refusals\)", cases)
if not m:
    fail("`heap test` prints no tally")
if m.group(1) != m.group(2):
    fail(f"only {m.group(1)} of {m.group(2)} bump checks held")
if (int(m.group(5)), int(m.group(4), 16)) != (hkib, hbase):
    fail(f"`heap test` tallies against a {m.group(5)} KiB reservation at {m.group(4)}, "
         f"`heap` reports {hkib} KiB at {hbase:#010x}")
if int(m.group(3)) != int(m.group(5)) + int(m.group(6)):
    fail(f"`heap test` says arena {m.group(3)} KiB but {m.group(5)} reserved + "
         f"{m.group(6)} grown")
if int(m.group(6)) == 0:
    fail("`heap test` grew nothing, yet one of its cases serves 1.5 MiB out of a 1 MiB "
         "reservation — the case passed without the growth it claims")
if int(m.group(8)) != 0:
    fail(f"{m.group(8)} live refusals during a self-test whose synthetic cases only call "
         f"the PURE place() — and whose one live growth was served, not refused")
checks = int(m.group(2))
tgrown = int(m.group(6))
# The tally is printed by the same boot that `mem` then reports, so the growth `heap test`
# performed must be the growth `mem` sees. A second, quieter growth path would show up here
# as two different figures for one arena.
if tgrown != mgrown:
    fail(f"`heap test` grew {tgrown} KiB, `mem` reports {mgrown} KiB grown on the same boot")

# --- part 6: the policy bound, LIVE (2.741) -----------------------------------------
# Everything in part 5 is arithmetic over synthetic pools. This is the kernel obeying it on
# the board: `heap grow 65536` asks for 64 MiB of a machine that has 222 MiB free, so the
# frame allocator would have said yes and only policy says no. Three things have to hold —
# the refusal names POLICY rather than out-of-memory, it happens before a frame moves, and
# the arena is exactly where `mem` and `pmap` just found it afterwards.
if "afrisovera> heap grow 65536" not in s:
    fail("`heap grow 65536` never ran — the live policy refusal is unproven")
tail = s.split("afrisovera> heap grow 65536")[-1]
m = re.search(r"\[heap grow\] REFUSED before a frame was touched: POLICY: (.+)", tail)
if not m:
    if "[heap grow] grew" in tail or "[heap grow] served" in tail:
        fail("the heap GREW to serve a 64 MiB ask — a quarter of the machine handed to one "
             "allocator, which is exactly what 2.741 exists to stop")
    fail("`heap grow 65536` was not refused BY POLICY on a 222 MiB board")
why = m.group(1).strip()
if "share of managed memory" not in why:
    fail(f"the 64 MiB ask was refused by the floor, not the cap: {why!r} — 64 MiB is over "
         f"a quarter of this board, so the CAP is the bound that should bind")
if "out of physical frames" in why:
    fail(f"a policy refusal reported as out-of-memory: {why!r} — the operator goes hunting "
         f"a leak on a machine with 222 MiB free")
# What it would have cost, against what policy leaves and against what the machine has. The
# middle figure is the milestone: cost fits in the pool and does NOT fit in the headroom.
m = re.search(r"\[heap grow\] it would have cost (\d+) frame\(s\); headroom is (\d+) "
              r"frame\(s\) of (\d+) free", tail)
if not m:
    fail("the refusal does not state what the ask would have cost against the headroom "
         "policy leaves — a bound with no figure is not actionable")
cost, head, freeb = int(m.group(1)), int(m.group(2)), int(m.group(3))
if cost <= head:
    fail(f"an ask costing {cost} frame(s) was refused with {head} frame(s) of headroom — "
         f"the refusal contradicts the row `heap` prints")
if cost > freeb:
    fail(f"the ask wanted {cost} frame(s) of {freeb} free: the MACHINE could not serve it "
         f"either, so this proves nothing about policy")
if head >= freeb:
    fail(f"policy left {head} frame(s) of the {freeb} free: the cap and floor are not "
         f"being subtracted on the live board")
# ...and nothing moved. Both allocators, on the far side of a refusal.
m = re.search(r"\[heap grow\] arena INTACT at (\d+) B \((\d+) B grown, (\d+) growth\(s\)\); "
              r"frames (\d+) -> (\d+) free", tail)
if not m:
    fail("the refusal does not state that the arena is intact — a growth that refuses "
         "after taking frames has still spent them")
aft, gaft, gruns, fb, fa = (int(m.group(i)) for i in range(1, 6))
if aft != mtotal or gaft != mgrown * 1024:
    fail(f"after the refusal the arena is {aft} B ({gaft} B grown); `mem` and `pmap` found "
         f"{mtotal} B ({mgrown * 1024} B grown) — the refusal kept something")
if fb != fa:
    fail(f"the frame pool went {fb} -> {fa} across a refusal that claims to have touched "
         f"no frame — {fb - fa} frame(s) spent on a request that was denied")
if gruns == 0:
    fail("the arena reports no growth at all by this point, so nothing here bounds growth")
# And the machine is still up to say so: the box printed after the refusal is the same box.
after = tail.split("afrisovera> heap")[-1]
m = re.search(r"^  can grow       (\d+) more frame\(s\)", after, re.M)
if not m:
    fail("the kernel did not survive a policy refusal well enough to print `heap` again")
if int(m.group(1)) >= freeb:
    fail(f"after the refusal `heap` offers {m.group(1)} frame(s) of {freeb} free again — "
         f"the row went back to quoting the pool")

print(f"HEAP TEST PASSED  (reservation {hbase:#010x}..{hend:#010x} = {hkib} KiB in .bss, "
      f"16-byte aligned and inside the loaded image; {used} B used ({pad} B of it "
      f"alignment padding), high-water {high} B — LEADING used by {high - used} B, "
      f"because dealloc now gives back; the arena tops at {hend:#010x}, below the frame "
      f"allocator's own "
      f"reported managed base {fbase:#010x}, so no frame overlaps the heap; mem and pmap "
      f"describe the SAME arena rather than a second opinion; exhaustion routes through "
      f"the pure place() and is refused BY NAME (arena / oversize-request / bad-align / "
      f"out-of-frames / unmapped-span) with a console line before the null, from exactly "
      f"one site; and the arena is no longer the reservation — `heap test` GREW it by "
      f"{tgrown} KiB in {pframes} frame(s) to serve a 1.5 MiB request no 1 MiB arena could, "
      f"which mem and pmap both report as {mkib} KiB = {mres} reserved + {mgrown} grown and "
      f"the frame allocator confirms as {pframes} of its own frames handed out; "
      f"{checks}/{checks} bump checks held against synthetic requests the live kernel never "
      f"makes — exact fit and one past it, alignment that exhausts what the size alone "
      f"would fit, a request bigger than the whole arena, the two-core race a lost CAS "
      f"re-places out of, and the growth width a frame count cannot wrap; and growth now "
      f"has a SHARE as well as its accidents — a live `heap grow 65536` costing {cost} "
      f"frame(s) was refused BY POLICY with {freeb} free next door and only {head} of them "
      f"the heap's to take, before a frame moved: {fb} -> {fa} frames and the arena still "
      f"exactly {aft} B)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "HEAP TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
