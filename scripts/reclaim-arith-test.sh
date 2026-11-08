#!/usr/bin/env bash
# Verify the arithmetic of GIVING MEMORY BACK, and that the arena now measures what it
# throws away (milestone 2.737, first half).
#
# 2.734-2.736 made arena exhaustion legible: the bound is named, the largest customer
# refuses through a caller, and an allocation panic states which bound refused and how
# full the arena was. None of it touches *why* the arena fills, which `heap.rs` stated in
# one line: `dealloc` was an empty body. The allocator is MONOTONIC — every `Vec`,
# `String` and `format!` the shell grows per command is handed out once and never
# returned — so the machine does not need a large request to die, it needs uptime, and
# 2.735's "room for N more stacks" silently decays under a user doing nothing wrong.
#
# The fix is a free list threaded through the freed blocks themselves. That list is the
# GLOBAL allocator, so a bug in it is not a failing test, it is a kernel that cannot
# boot. This milestone therefore lands the two halves that can be proved without putting
# an untested allocator under every allocation in the kernel:
#
#   * the POLICY, as pure total functions — `heap::reclaim_span` (which returned blocks
#     can carry a list node, and what alignment trimming costs), `heap::carve` (where a
#     request lands inside a free block and what survives on either side), and
#     `heap::adjacent` (the whole of coalescing) — driven by a free list over a fixed
#     array in `heap test`, so every decision the real list will make is executed here
#     first, including the property a bump allocator fails: an alloc/free cycle repeated
#     far past the arena's size must NOT exhaust it;
#   * the MEASUREMENT — `dealloc` now counts the blocks and bytes handed back and how
#     much of that a free list could actually take, so "the arena decays under uptime"
#     stops being an assertion and becomes a figure on the console.
#
# Threading the nodes through live memory and switching `GlobalAlloc` onto them is 2.738.
# What this suite must therefore ALSO check is that it has not happened yet by accident:
# the reported figures must be consistent, and the live kernel must still boot.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"

cargo build -p mansa-kernel || { echo "RECLAIM ARITH TEST FAILED (build)"; exit 1; }

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:4872,server,nowait \
    -qmp tcp:127.0.0.1:4873,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/reclaimarith.out"
# `echo` first so the shell has certainly grown (and dropped) a buffer or two before the
# counters below are read: a suite that reported "0 B returned" would be proving nothing
# about a `dealloc` that is called.
python3 scripts/shell-demo.py 4872 4873 "${TMPDIR:-/tmp}/reclaimarith.png" \
    "echo warmup" "ls" "heap test" "heap" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bump checks held|reclaimable|\[ok\]   (a|an|the|two|eight|and)|\[FAIL\]|returned " \
    "$OUT" | sed 's/^/  /'

python3 - "$OUT" kernel/src/heap.rs <<'PY'
import re, sys
s    = open(sys.argv[1], "rb").read().decode("latin1")
heap = open(sys.argv[2], encoding="utf-8").read()

def fail(msg):
    print(f"RECLAIM ARITH TEST FAILED ({msg})"); sys.exit(1)

def dash(t):
    """Every way an em dash can reach this comparison, folded to one ASCII marker.

    2.736's lesson, kept: the case names below carry their REASON after the dash, which
    is the half worth checking, and the serial capture is whatever encoding
    `shell-demo.py`'s stdout gave it (cp1252 on this host, UTF-8 elsewhere). Reading the
    capture byte-preserving and folding BOTH sides is the only version that does not
    depend on the machine the suite runs on. The needles are built from code points
    rather than typed so re-saving this file cannot quietly break one of them."""
    for needle in (chr(0x2014), chr(0x97), chr(0xE2) + chr(0x80) + chr(0x94)):
        t = t.replace(needle, "-")
    return t

def code(src):
    """The source minus its line comments — 2.735's helper for 2.735's reason: this
    milestone's doc-comments quote the empty `dealloc` body they replaced, and a grep
    that cannot tell a citation from a call would force the history to go unwritten."""
    return "\n".join(ln for ln in src.splitlines() if not ln.lstrip().startswith("//"))

heap = code(heap)

# --- part 1: the policy exists as pure, testable functions ---------------------------
# The reason this milestone is worth its own step: an allocator whose decisions are only
# reachable through `unsafe` pointer work cannot be proved before it is trusted.
for sig in ("pub fn reclaim_span(ptr: usize, size: usize) -> Option<Span>",
            "pub fn carve(start: usize, len: usize, size: usize, align: usize) -> Option<Carve>",
            "pub fn adjacent(a_start: usize, a_len: usize, b_start: usize) -> bool"):
    if sig not in heap:
        fail(f"missing pure policy function: {sig.split('(')[0]}")
# Total, like `place` before them: no argument may wrap or panic.
for fn, needed in (("reclaim_span", "checked_add"), ("carve", "checked_add"),
                   ("adjacent", "checked_add")):
    body = heap.split(f"pub fn {fn}(")[-1].split("\npub fn ")[0]
    if needed not in body:
        fail(f"{fn} does not use {needed} — a run near the top of the address space "
             f"would wrap into a false answer instead of refusing")
# The node's size is derived from what a node actually holds, not typed as a magic 16.
if "pub const NODE_SIZE: usize = 2 * core::mem::size_of::<usize>()" not in heap:
    fail("NODE_SIZE is not derived from the two words a node holds")

# --- part 2: dealloc measures what it discards ---------------------------------------
d = heap.split("unsafe fn dealloc(")[-1].split("\n    }")[0]
if "_ptr" in d or "_layout" in d:
    fail("dealloc still ignores its arguments — the decay stays unmeasured")
for needed in ("FREES.fetch_add", "FREED.fetch_add", "reclaim_span("):
    if needed not in d:
        fail(f"dealloc does not record {needed} — 'the arena decays under uptime' is "
             f"then an assertion, not a figure anyone can read")
for acc in ("pub fn frees()", "pub fn freed()", "pub fn reclaimable()"):
    if acc not in heap:
        fail(f"{acc} is not published, so the shell cannot report the decay")
# NOT yet reclaiming: this milestone is the arithmetic and the measurement. A dealloc
# that had quietly started handing bytes back out would make every figure below a
# different claim than the one this suite checks.
if "USED.fetch_sub" in d or "USED.store" in d:
    fail("dealloc mutates USED — the free list landing early is 2.738, and it would not "
         "be proved by this suite")

# --- part 3: the policy, against cases the live kernel never makes -------------------
bump = s.split("kernel heap: bump self-test against synthetic requests")[-1]
for phrase in (
        # reclaim_span: which frees are winnable at all.
        "an aligned block long enough for a node is reclaimed whole, losing nothing",
        "an UNALIGNED block trims forward to the node alignment, and the trim is counted",
        "a block shorter than one node is NOT reclaimable — refused, not counted as won",
        "trimming can itself make a block too short, and a zero-byte free reclaims nothing",
        "a block that would run past the top of the address space refuses, never wraps",
        # carve: the split.
        "a request at an aligned block starts at its base and leaves the rest as one node",
        "an alignment gap big enough to be a node is KEPT as one, not absorbed into the grant",
        "a gap too short for a node is charged as loss, not left as an unusable entry",
        "a tail remainder shorter than a node is charged the same way, on the same rule",
        "a block too small for the request — or for its alignment — is not a fit, so the scan moves on",
        "a non-power-of-two alignment is refused here too, rather than masked into a wild address",
        # coalescing.
        "two runs merge only if they exactly touch, and a run at the top does not wrap into one",
        "two separated frees stay two nodes — the list does not pretend a hole is not there",
        "a free between two free neighbours collapses all THREE into one run, in one insert",
        "and the coalesced run then serves a request no one of its pieces could have",
        # the property the whole milestone is for.
        "the BUMP exhausts a 4 KiB arena after 8 grants, no matter that each was freed",
        "the free list runs the same alloc/free cycle 1000 times",
        "and ends with the arena back as ONE whole run: 1000 cycles fragmented nothing",
        "a full arena refuses through the free list too — reclaiming is not overcommitting",
        "eight blocks released out of order coalesce back to the whole arena, not eight crumbs",
        "the returned/reclaimable pair is consistent: never more won than was handed back"):
    if dash(f"[ok]   {phrase}") not in dash(bump):
        fail(f"reclaim policy not proved: {phrase}")
if "[FAIL]" in s:
    fail("a check did not hold")
m = re.search(r"kernel heap: (\d+)/(\d+) bump checks held", bump)
if not m or m.group(1) != m.group(2):
    fail("the heap's checks did not all hold")
checks = int(m.group(2))
if checks < 40:
    fail(f"only {checks} heap checks ran — 2.736 already ran 25, so the reclaim cases "
         f"did not reach the self-test")

# --- part 4: the decay, as a live figure ---------------------------------------------
# The counters must be real, not zeroes that would pass part 2's grep and prove nothing:
# the shell grows and drops buffers per command, so a boot that ran five commands and
# reports no frees means `dealloc` is not on the path the checks above assume.
m = re.search(r"kernel heap: (\d+) B returned in (\d+) block\(s\) so far, (\d+) B of it "
              r"reclaimable", s)
if not m:
    fail("`heap test` does not report the returned/reclaimable figures")
freed, frees, reclaimable = (int(m.group(i)) for i in (1, 2, 3))
if frees == 0 or freed == 0:
    fail("the kernel reports NO frees after five shell commands — dealloc is not being "
         "called, so the decay this milestone measures is not the one it describes")
if reclaimable > freed:
    fail(f"reclaimable ({reclaimable} B) exceeds the bytes returned ({freed} B) — the "
         f"pair is not measuring the same thing")
# 2.742: bound `freed` by the arena AS THE KERNEL REPORTS IT. This line carried a `1 << 20`
# literal from 2.736, when the arena WAS 1 MiB of `.bss` and could be nothing else. Since
# 2.740 the arena is that reservation plus every byte grown into it from `frame::`, and this
# suite went RED on a boot that returned 1703088 B out of a 2564 KiB grown arena — the
# counter being RIGHT, and 2.740 working. A literal here does not test the counter; it tests
# that the arena never grew. `heap` prints both halves of the split on its `grown` row.
m = re.search(r"arena is (\d+) B, not the (\d+) B reserved", s)
if not m:
    fail("`heap` does not report the reserved + grown arena split, so this suite has no "
         "figure to bound the counters by other than a literal — which is the 2.742 defect")
heap_arena, heap_reserved = int(m.group(1)), int(m.group(2))
if heap_arena < heap_reserved:
    fail(f"the arena ({heap_arena} B) is smaller than its own reservation ({heap_reserved} B)")
if freed >= heap_arena:
    fail(f"{freed} B reported returned from a {heap_arena} B arena — the counter is wrong")
# 2.742: these three needles were written in 2.736, when every returned byte was DISCARDED
# and the winnable part was only a projection ("could carry a free node"). 2.739 put the
# kernel's own allocator on the free list, so the rows now report bytes actually RECLAIMED
# and requests actually served out of them. The CLAIM each assertion makes is unchanged —
# the operator can read the decay off `heap` without running the self-test, the winnable
# part is stated beside it, and `mem` carries the same figure — but each is matched against
# the sentence the kernel writes today rather than the one it wrote two milestones ago.
if not re.search(r"^  returned       \d+ B in \d+ block\(s\) - \d+ B of it RECLAIMED and free again",
                 dash(s), re.M):
    fail("`heap` does not state the decay, so an operator still cannot read it without "
         "running the self-test — 2.732's complaint again")
if not re.search(r"^  reused         \d+ request\(s\) served out of returned memory \(\d+ B\)",
                 dash(s), re.M):
    fail("`heap` states the bytes returned but not the part the free list actually won back")
if not re.search(r"B returned in \d+ block\(s\), \d+ B of it free again \(\d+ reuse\(s\), "
                 r"bump at \d+ B\)", dash(s)):
    fail("`mem` does not carry the same figure, so the two reports disagree")

# --- part 5: the live kernel is untouched --------------------------------------------
# Every change above is additive by design. If the machine stopped booting, or the arena
# started refusing, the measurement broke the thing it was measuring.
if "KERNEL PANIC" in s:
    fail("the kernel panicked on an ordinary boot")
if "[heap] REFUSED" in s:
    fail("the arena refused a request on an ordinary boot")
m = re.search(r"^  used           (\d+) B", s, re.M)
if not m:
    fail("`heap` does not report used bytes")
used = int(m.group(1))
if used == 0 or used >= heap_arena:
    fail(f"the arena reports {used} B used of {heap_arena} B — not a live figure")

print(f"RECLAIM ARITH TEST PASSED  (the arena's monotonicity now has both a MEASURE and "
      f"a REMEDY that has been executed: `dealloc` was an empty body, so the claim 'the "
      f"machine's ceiling is measured in uptime' was true and unmeasured — this boot "
      f"hands back {freed} B in {frees} block(s) over five shell commands, of which "
      f"{reclaimable} B is reclaimable — and since 2.739 is RECLAIMED rather than "
      f"discarded, out of an arena `heap` itself reports as {heap_arena} B ({heap_reserved} "
      f"B reserved + the rest grown from frames since 2.740) rather than the 1 MiB literal "
      f"this suite asserted against until 2.742; and `heap`/`mem` state that instead of "
      f"only the {used} B used that can never fall; and the "
      f"policy that will stop the decay is landed as pure total functions — reclaim_span "
      f"(which frees are winnable, and what alignment trimming costs), carve (where a "
      f"request lands in a free block, and which remainders are big enough to stay "
      f"nodes rather than be charged as loss), adjacent (the whole of coalescing) — "
      f"driven by a first-fit address-ordered free list in `heap test` that proves the "
      f"property a bump allocator fails: the bump dies after 8 grants of a 4 KiB arena "
      f"however promptly each is freed, while the list runs the SAME alloc/free cycle "
      f"1000 times, 125x the arena, and ends with it back as one whole run; eight blocks "
      f"released out of order coalesce back to the whole arena rather than eight "
      f"unusable crumbs; {checks}/{checks} heap checks held, and USED is still never "
      f"decremented, because threading the nodes through live memory under every "
      f"allocation in the kernel is 2.738 and is not proved by this suite)")
PY
rc=$?
if [ $rc -ne 0 ]; then
    echo "RECLAIM ARITH TEST FAILED (see assertions above)"
    exit 1
fi
exit 0
