#!/usr/bin/env bash
# Verify the free list in REAL MEMORY — nodes threaded through the freed blocks themselves
# (2.738) — and that the kernel's own `GlobalAlloc` now runs on it (2.739).
#
# 2.737 landed the two halves of reclaiming that could be proved with the bump still under
# every allocation in the kernel: the MEASUREMENT (`dealloc` counts the blocks and bytes
# handed back, and how much of that a free list could take) and the POLICY as pure total
# functions (`reclaim_span`, `carve`, `adjacent`), driven by a first-fit address-ordered
# list over a FIXED ARRAY. That array was the right first step and it is not the same
# claim: an array cannot get a pointer wrong, cannot have a node overwritten by the caller
# it just served, and cannot lose the list to a grant carved out from under it.
#
# This milestone removes the array. `heap::FreeList` is the same algorithm — every decision
# still comes from the three pure functions — with its two-word nodes written INTO the free
# blocks, so the bookkeeping needs no side table and therefore cannot itself need the
# allocator. It is driven over a real, private 4 KiB `.bss` arena by `heap test`.
#
# This suite's part 2 used to check what had deliberately NOT happened — that `GlobalAlloc`
# was still the bump, because a list under test that had quietly become the kernel's
# allocator would have been an untested allocator under every `format!` on the boot path.
# 2.739 made the switch, on the strength of exactly the cases below, so part 2 now checks
# the other half of that same contract: `GlobalAlloc` is on the list, and the list under it
# is still the one these cases drive. The switch's OWN proof — `used` FALLING on the live
# kernel arena, the bump cursor not moving across 200 cycles, and a `Vec` dropped by
# ordinary kernel code becoming the next `Vec`'s memory — is part 5, and it is checked
# here rather than in a suite of its own precisely because it is the same list: a proof
# of the structure that did not also prove the kernel runs on it would be proving a
# rehearsal.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "FREELIST LIVE TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:4874,server,nowait \
    -qmp tcp:127.0.0.1:4875,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/freelistlive.out"
# `heap test` twice: the list is built fresh from a `FreeList::new()` each run, so a second
# pass proves the arena was left in a state the next run can start from — a list that had
# corrupted its own memory would pass once and fail here.
python3 scripts/shell-demo.py 4874 4875 "${TMPDIR:-/tmp}/freelistlive.png" \
    "echo warmup" "heap test" "heap" "heap test" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bump checks held|free list proved|LIVE arena|\[ok\]   (the|a|an|and|eight|six|released|200|after)|\[FAIL\]" \
    "$OUT" | sed 's/^/  /'

python3 - "$OUT" kernel/src/heap.rs <<'PY'
import re, sys
s    = open(sys.argv[1], "rb").read().decode("latin1")
heap = open(sys.argv[2], encoding="utf-8").read()

def fail(msg):
    print(f"FREELIST LIVE TEST FAILED ({msg})"); sys.exit(1)

def dash(t):
    """2.736's helper, kept for 2.736's reason: the case names carry their REASON after an
    em dash, and the serial capture is in whatever encoding `shell-demo.py`'s stdout gave
    it (cp1252 on this host, UTF-8 elsewhere). Fold BOTH sides, from code points rather
    than typed characters, so re-saving this file cannot quietly break a needle."""
    for needle in (chr(0x2014), chr(0x97), chr(0xE2) + chr(0x80) + chr(0x94)):
        t = t.replace(needle, "-")
    return t

def code(src):
    """Source minus line comments — this file's doc-comments describe the array the list
    replaces and the GlobalAlloc switch it does NOT make, and a grep that cannot tell a
    citation from a call would force the history to go unwritten."""
    return "\n".join(ln for ln in src.splitlines() if not ln.lstrip().startswith("//"))

heap = code(heap)

# --- part 1: the list exists, and its nodes are the memory ---------------------------
if "struct Node {" not in heap or "next: *mut Node" not in heap or "len: usize" not in heap:
    fail("no two-word Node written into the block itself")
if "#[repr(C)]\nstruct Node" not in heap:
    fail("Node is not repr(C) — NODE_SIZE and reclaim_span were written against a layout "
         "the compiler would then be free to change")
for sig in ("pub unsafe fn insert(&mut self, start: usize, len: usize)",
            "pub unsafe fn take(&mut self, size: usize, align: usize) -> Option<usize>",
            "pub unsafe fn give_back(&mut self, ptr: usize, size: usize)",
            "pub unsafe fn intact(&self, lo: usize, hi: usize) -> bool"):
    if sig not in heap:
        fail(f"FreeList is missing {sig.split('(')[0]}")
# The point of the milestone: the same three pure functions decide everything, so 2.737's
# 45 arithmetic checks still cover the policy and only the pointer work is new.
body = heap.split("impl FreeList {")[-1].split("\n}")[0]
for f in ("carve(", "reclaim_span(", "adjacent("):
    if f not in body:
        fail(f"FreeList does not call {f} — the list has its own copy of the policy, so "
             f"2.737's proof no longer covers it")
# No side table anywhere: the list is a head pointer and a loss counter, nothing else.
if re.search(r"struct FreeList \{\s*head: \*mut Node,\s*/?", heap) is None:
    fail("FreeList does not start from a bare head pointer")
if "[(usize, usize);" in body or "spans" in body:
    fail("FreeList carries an array — the node-in-the-block property is what this "
         "milestone is for")
# The bug this milestone found, pinned. `carve`'s tail starts at `at + size`, so a request
# whose size is not a multiple of NODE_ALIGN leaves its remainder misaligned. An array can
# hold such an address; a node written there is a misaligned dereference. Both remainders
# must therefore go back through `reclaim_span` (via `give_back`), which trims and charges.
# Split on the next `pub` and not on a doc comment: `code()` above has already stripped
# every `///` line, so a doc-comment needle would silently swallow the rest of the impl.
take = body.split("pub unsafe fn take(")[-1].split("\n    pub ")[0]
if "self.insert(" in take:
    fail("FreeList::take re-inserts a carve remainder directly — carve does not promise a "
         "node-ALIGNED start, only a long-enough one, so this writes a misaligned node")
if take.count("self.give_back(") != 2:
    fail("FreeList::take does not return both carve remainders through give_back/reclaim_span")

# --- part 2: GlobalAlloc IS the list now (2.739) --------------------------------------
# The switch these cases were the rehearsal for. What must hold is that the allocator the
# kernel runs on is THIS list — not a second copy of the policy that these cases would
# then say nothing about.
if "unsafe impl GlobalAlloc for KernelAlloc {" not in heap:
    fail("no `impl GlobalAlloc for KernelAlloc` — 2.739 put the kernel's allocator on the "
         "free list, and a suite that cannot find it is testing something else")
ga = heap.split("unsafe impl GlobalAlloc for KernelAlloc {")[-1].split("\n}")[0]
a = ga.split("unsafe fn alloc(")[-1].split("unsafe fn dealloc(")[0]
if "list_take(" not in a or a.index("list_take(") > a.index("place("):
    fail("alloc does not try the free list BEFORE it bumps — the bump is the fallback now, "
         "not the first answer")
d = ga.split("unsafe fn dealloc(")[-1]
if "list_give(" not in d:
    fail("dealloc does not hand the block back to the list — the whole milestone")
if "USED.fetch_update" not in d and "USED.fetch_sub" not in d:
    fail("dealloc does not lower USED — a reclaiming allocator whose `used` still only "
         "rises is reporting a bump it no longer is")
# `list_take`/`list_give` must go through the same `FreeList` these cases drive.
# `with_list` is generic over the closure's return, so its needle stops before the `<R>` —
# a `(` here would be looking for a signature the source cannot have.
for f in ("fn list_take(", "fn list_give(", "fn with_list"):
    if f not in heap:
        fail(f"missing {f} — the allocator reaches the list some other way than the one "
             f"this suite proved")
if "static ARENA_LIST: ListCell" not in heap:
    fail("no single kernel-arena FreeList behind a lock")

# --- part 3: the list, driven over real memory ---------------------------------------
run = s.split("kernel heap: bump self-test against synthetic requests")[-1]
for phrase in (
        "the list's bookkeeping is IN the freed block — no side table, so it cannot itself need the allocator",
        "a one-run list over the whole arena satisfies every invariant the algorithm assumes",
        "the REAL list runs 1000 alloc/free cycles through a 4 KiB arena — 125x its size — writing over every grant",
        "and the arena is back as ONE whole run with nothing lost: the nodes survived every write",
        "eight simultaneous grants each still hold their own bytes — no two overlap, and no node sits inside one",
        "a fully granted arena refuses through the list too — reclaiming is not overcommitting",
        "released out of order, the eight coalesce back to the whole arena through real pointers",
        "six grants of mixed size and alignment are each honoured, and the list stays well-formed",
        "a request whose SIZE is not node-aligned leaves its remainder trimmed and charged, never a misaligned node",
        "an irregular pattern released out of order returns every byte not CHARGED as loss - two charges, so three runs",
        "the largest run still exceeds all six grants put together, and really serves a request that size",
        # 2.739: the same list, now under the kernel itself.
        "the free-list counters match a REAL traversal of the live list - the report is not a story about a second list",
        "a grant from the LIVE arena raises used, and returning it LOWERS used - what a bump can never do",
        "200 alloc/free cycles through the kernel's own arena move the bump cursor NOT ONE BYTE - every one served from returned memory",
        "a Vec dropped by ORDINARY kernel code frees into the list, and the next Vec of that shape is served out of it",
        # 2.740 reworded this line when the arena stopped being a link-time constant;
        # 2.742 follows the kernel's wording rather than the wording of the milestone
        # that first wrote it. The claim is unchanged — it is the arena that GREW.
        "after every live cycle above, the GROWN arena STILL adds up and the counters still match the walk"):
    if dash(f"[ok]   {phrase}") not in dash(run):
        fail(f"free list not proved in real memory: {phrase}")
if "[FAIL]" in s:
    fail("a check did not hold")

# 2.737's arithmetic must still be there and still hold: this milestone changes where the
# nodes live, not what the policy decides.
runs = re.findall(r"kernel heap: (\d+)/(\d+) bump checks held", s)
if len(runs) < 2:
    fail(f"`heap test` reported {len(runs)} time(s), expected 2 — the second run is the "
         f"proof that the first left the arena usable")
for got, want in runs:
    if got != want:
        fail(f"only {got}/{want} heap checks held")
checks = int(runs[0][1])
if checks < 55:
    fail(f"only {checks} heap checks ran — 2.737 already ran 45, so the live-memory cases "
         f"did not reach the self-test")
if runs[0] != runs[1]:
    fail("the two `heap test` runs disagree — the list left state behind, which under "
         "GlobalAlloc would be memory handed out twice")

# The needle is folded on BOTH sides: `dash()` has already turned the serial capture's em
# dash into "-", so a pattern still carrying a typed em dash could never match it.
m = re.search(r"kernel heap: free list proved over (\d+) B of REAL memory - (\d+) "
              r"alloc/free cycles, nodes threaded through the blocks, (\d+) B charged as "
              r"loss", dash(s))
if not m:
    fail("`heap test` does not report the live-memory run")
arena, cycles, lost = (int(m.group(i)) for i in (1, 2, 3))
if cycles != 1000:
    fail(f"the list completed {cycles} cycles, not 1000 — it exhausted a {arena} B arena, "
         f"which is the bump's failure and the whole reason for the list")
if lost > arena:
    fail(f"{lost} B charged as loss out of a {arena} B arena — the accounting is wrong")
if "GlobalAlloc now reclaims" not in s:
    fail("the self-test does not state that the kernel's allocator is the list")

# --- part 4: the live kernel is untouched --------------------------------------------
# Every change is additive: a new type, a private arena, and cases in `heap test`. If the
# machine stopped booting or the arena started refusing, the rehearsal broke the stage.
if "KERNEL PANIC" in s:
    fail("the kernel panicked on an ordinary boot")
if "[heap] REFUSED" in s:
    fail("the arena refused a request on an ordinary boot")
m = re.search(r"^  used           (\d+) B", s, re.M)
if not m:
    fail("`heap` does not report used bytes")
used = int(m.group(1))
# 2.742: the ceiling `used` is checked against is the arena AS THE KERNEL REPORTS IT, not
# the 1 MiB literal this line carried since 2.737. Since 2.740 the arena is the .bss
# reservation PLUS every byte grown into it from `frame::`, so a hard-coded `1 << 20` here
# does not test the allocator — it tests that the allocator never grew, and would red on a
# healthy boot the moment it did. `heap` prints both halves of the split on the `grown` row.
m = re.search(r"arena is (\d+) B, not the (\d+) B reserved", s)
if not m:
    fail("`heap` does not report the reserved + grown arena split, so this suite has no "
         "figure to bound `used` by other than a literal — which is the 2.742 defect")
heap_arena, heap_reserved = int(m.group(1)), int(m.group(2))
if heap_arena < heap_reserved:
    fail(f"the arena ({heap_arena} B) is smaller than its own reservation ({heap_reserved} B)")
if used == 0 or used >= heap_arena:
    fail(f"the arena reports {used} B used of {heap_arena} B — not a live figure")
m = re.search(r"^  returned       (\d+) B in \d+ block\(s\) - (\d+) B of it RECLAIMED and free again",
              dash(s), re.M)
if not m:
    fail("`heap` no longer states the decay 2.737 made readable, now as the part of it that "
         "is free again")
returned, reclaimed = int(m.group(1)), int(m.group(2))
if returned == 0:
    fail("nothing was returned on an ordinary boot — the live cases below would then be "
         "proving the list over an arena nobody freed into")

# --- part 5: the LIVE kernel arena reclaims (2.739's own proof) -----------------------
# Part 3 proves the structure over a private test arena. This part is the reason that
# mattered: the SAME list is under `GlobalAlloc`, so the kernel's own arena must show the
# one behaviour a bump cannot produce — `used` going DOWN.
m = re.search(r"^kernel heap: LIVE arena - (\d+) request\(s\) served from returned memory "
              r"\((\d+) B\), bump cursor at (\d+) B while used is (\d+) B, (\d+) B in the "
              r"list, (\d+) B lost, (\d+) missed free\(s\)", dash(s), re.M)
if not m:
    fail("`heap test` does not report the LIVE arena — the switch is unmeasured on the very "
         "arena it was made for")
served, served_b, cursor, live_used, in_list, lost_b, missed = (int(m.group(i)) for i in range(1, 8))
if served < 200:
    fail(f"only {served} request(s) served out of returned memory — the 200-cycle case alone "
         f"should account for that many, so the list is not actually feeding `alloc`")
if live_used >= cursor:
    fail(f"used ({live_used} B) is not BELOW the bump cursor ({cursor} B) — under a real "
         f"reclaiming allocator the gap IS the reclaimed memory, and no gap means the list "
         f"is being written to and never read from")
# 2.742: the arena's identity, restated for an arena that GROWS. Until 2.740 every byte the
# allocator had ever made available came from one place — the bump walking the .bss
# reservation — so "cursor = used + list" was the whole of it. Since 2.740 a byte can also
# enter the arena as a FRAME donated by `frame::`, and those bytes join the list without the
# cursor moving one step, which is the very property the growth cases above prove. So the
# left-hand side is now everything ever made available (bumped + grown) and the right-hand
# side is still everything accounted for (held + free). The claim is UNCHANGED and just as
# strict: a byte in the arena is either held by a caller or on the free list, and any
# discrepancy is still memory lost silently or hand-outable twice.
grown = heap_arena - heap_reserved
if cursor + grown != live_used + in_list:
    fail(f"the arena does not add up: cursor {cursor} B + {grown} B grown != used "
         f"{live_used} B + list {in_list} B — bytes the allocator has made available are "
         f"neither held nor free, so some are either lost silently or hand-outable twice")
if missed > 0:
    fail(f"{missed} free(s) were dropped because the list was locked — on a single-CPU boot "
         f"that is a re-entrant free, not contention")
m = re.search(r"^  high-water     (\d+) B - used has since fallen by (\d+) B", dash(s), re.M)
if not m or int(m.group(2)) == 0:
    fail("`heap` reports no fall from the high-water mark — the headline claim of the "
         "milestone, and the only one an operator ever sees")
fell = int(m.group(2))

print(f"FREELIST LIVE TEST PASSED  (the free list now exists in REAL MEMORY: 2.737 proved "
      f"the policy over a fixed array, which cannot get a pointer wrong, cannot have a "
      f"node overwritten by the caller it just served and cannot lose the list to a grant "
      f"carved out from under it — `heap::FreeList` is the same algorithm with the array "
      f"removed, its two-word nodes written into the free blocks themselves so the "
      f"bookkeeping needs no side table and cannot need the allocator, and every decision "
      f"still taken by reclaim_span/carve/adjacent so 2.737's arithmetic still covers the "
      f"policy and only the pointer work is new; over a private {arena} B arena it ran "
      f"{cycles} alloc/free cycles — {cycles * 512 // arena}x the arena — WRITING OVER "
      f"every grant, and ended with it back as one whole run; eight simultaneous grants "
      f"each still held their own bytes, so no two overlapped and no node ever sat inside "
      f"a live one; released out of order they coalesced back to the whole arena, and an "
      f"irregular six-grant pattern of mixed size and alignment came back as three runs "
      f"and not one — honestly, because two of its carves charged a loss ({lost} B total: "
      f"an 8 B alignment pad and a 4 B tail off a 300 B request) and a charged byte is a "
      f"HOLE no coalescing can close, which is exactly why the policy charges it instead "
      f"of absorbing it into a run it would then hand out as whole — with every remaining "
      f"byte in a run, the invariants intact after every phase, and the largest single run "
      f"still longer than all six grants put together; and {checks}/{checks} heap checks "
      f"held on BOTH of two consecutive runs. THE SWITCH IS MADE: `GlobalAlloc::alloc` "
      f"asks the list before it bumps and `dealloc` hands the block back, so on the "
      f"kernel's OWN arena a grant raises `used` and returning it LOWERS it — the one "
      f"thing a bump can never do — {served} request(s) ({served_b} B) were served out of "
      f"returned memory rather than by bumping, 200 alloc/free cycles each writing over "
      f"their grant moved the bump cursor NOT ONE BYTE, and a `Vec` dropped by ordinary "
      f"kernel code became the next `Vec`'s memory, which is the claim that matters "
      f"because this is the allocator under every format! on the boot path; the arena "
      f"adds up at the end — and adds up as an arena that GREW, {cursor} B cursor + "
      f"{grown} B grown from frames = {live_used} B used + {in_list} B in the list "
      f"(a {heap_arena} B arena over a {heap_reserved} B reservation, {lost_b} B lost, "
      f"{missed} free(s) missed) — the counters match a REAL "
      f"traversal of the live list rather than a story about a second one, and `heap` now "
      f"reports used FALLEN {fell} B below its high-water mark on nothing but an ordinary "
      f"boot)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "FREELIST LIVE TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
