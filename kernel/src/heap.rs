//! Kernel heap — a minimal bump allocator backing the `alloc` crate.
//!
//! This unlocks `Box`/`Vec`/`String` inside the kernel (used, e.g., to allocate
//! thread stacks). It is intentionally the simplest allocator that is *correct*:
//! it bumps a pointer forward and never reclaims (`dealloc` is a no-op). That is
//! fine for Phase 0.2's small, long-lived allocations; a real free-list / slab
//! allocator replaces it before userspace churns memory in Phase 0.3+.
//!
//! 2.734: the arena's *placement* stays compile-time — it lives in the image, and is
//! reserved before there is a tree to parse — but its *bound* no longer does. 2.731
//! made the frame allocator's ceilings visible before they fire and named when they do;
//! this file was the third place the same complaint applied. Exhaustion used to return
//! a null pointer that surfaced wherever the caller happened to `unwrap`, with nothing
//! attached saying it was the heap that ran out. Now the arena is reported where the
//! other two ceilings are (`heap`/`mem`/`pmap`), the refusal is a *named* bound, and
//! [`place`] — the bump arithmetic itself — is pure, so `heap test` can put requests in
//! front of it that the live kernel never makes.
//!
//! 2.739: and the allocator stops being a bump. 2.737 proved the free list's arithmetic
//! over a fixed array and 2.738 threaded its nodes through real memory in a private arena;
//! this milestone puts [`GlobalAlloc`] on that list. `alloc` tries the list before it
//! bumps — the bump stays the fast path for a never-freed arena and the fallback when
//! nothing fits — `dealloc` hands the block back, and `used` therefore FALLS. Everything
//! 2.734-2.736 made legible (`free`, `high_water`, `stacks_left()`) stops decaying under
//! nothing but uptime, which was the point of naming those bounds in the first place.
//!
//! 2.740: and the arena stops being a link-time constant. Reclaiming retires "it only ever
//! fills"; it does nothing about the older objection, which is that the reservation is
//! **1 MiB** while [`crate::frame`] manages 222 MiB next door — so a workload legitimately
//! holding 2 MiB live still dies against a ceiling no operator can raise. On a refusal the
//! list genuinely cannot satisfy, the arena now takes frames from `frame::alloc()` and hands
//! each span to [`FreeList::give_back`]. That is the whole trick: a grown arena is not new
//! machinery but the one thing the list already does — [`reclaim_span`] decides whether a
//! node can live in the span, [`adjacent`] coalesces a frame that happens to touch an
//! existing run — so the growth path adds a *source* of memory and not a second allocator.
//! The bump still runs over the `.bss` reservation and only over it ([`size`]); what grows
//! is [`arena`], and `used + free == arena` holds with a variable on the right.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::{addr_of_mut, null_mut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 1 MiB kernel heap, reserved in `.bss`.
const HEAP_SIZE: usize = 1 << 20;

#[repr(align(16))]
#[allow(dead_code)] // accessed only via addr_of_mut! as a raw byte region
struct HeapArena([u8; HEAP_SIZE]);
static mut HEAP: HeapArena = HeapArena([0; HEAP_SIZE]);

/// A second, private arena the free list is driven over by `heap test` (2.738).
///
/// Real memory, deliberately NOT the kernel's: the point of this milestone is that the
/// list's nodes are threaded through the blocks themselves, which only a real address
/// range can prove — but a list under test must not be able to corrupt the arena every
/// allocation in the kernel comes from. 4 KiB of `.bss` for the property that a bump
/// allocator cannot pass is a cheap trade, and it disappears the day the list *is* the
/// allocator and the test can point at a sub-range of `HEAP` instead.
///
/// The test's exact loss figures (the irregular pattern's `8 + 4` charged, three runs)
/// depend on where the arena starts MODULO 64, the largest alignment it asks for — and
/// `align(16)` alone left that to the linker's `.bss` order, which differs between a
/// Windows-hosted and a Linux-hosted build of the same commit: a Linux build put it at
/// 48 mod 64, where those figures are different (measured 1 Nov 2026: 0, 16, 32 pass,
/// 48 fails). So the arena is 64-aligned and the test starts `TEST_ARENA_SKEW` into it,
/// pinning the 16-mod-64 start every earlier green run measured, on every host.
const TEST_ARENA_SIZE: usize = 4096;
const TEST_ARENA_SKEW: usize = 16;
#[repr(align(64))]
#[allow(dead_code)] // accessed only via addr_of_mut! as a raw byte region
struct TestArena([u8; TEST_ARENA_SIZE + TEST_ARENA_SKEW]);
static mut TEST_ARENA: TestArena = TestArena([0; TEST_ARENA_SIZE + TEST_ARENA_SKEW]);

/// Why a request was refused. Two constants rather than a message built at the failure
/// site, for the reason `frame::BOUND_*` are two: the operator is owed the *difference*.
/// One is a transient state of this boot — the arena filled up, and a smaller request or
/// a bigger arena would have been served. The other can never be served by any arena
/// state, because the request is larger than the whole reservation; that is a bug in the
/// caller, not pressure on the heap.
pub const BOUND_ARENA: &str =
    "the arena has no room for it, and growing it did not find any (heap::arena() — a .bss reservation plus every frame since grown into it)";
pub const BOUND_REQUEST: &str = "the request is larger than the whole arena (no arena state serves it)";
/// `Layout` guarantees a power-of-two alignment, so this is unreachable from `alloc`.
/// It exists because [`place`] is public and total: a caller that hand-builds a request
/// gets a named refusal rather than a mask over a non-power-of-two and a wild pointer.
pub const BOUND_ALIGN: &str = "the requested alignment is not a power of two";
/// Growth bounds (2.740). A refusal that survives growth is a refusal against a DIFFERENT
/// allocator, and 2.736's message must say which: an operator told "the arena is full" when
/// the machine is in fact out of physical frames would go looking for a heap leak, and one
/// told "out of frames" when the growth region was never mapped would go looking for RAM.
pub const BOUND_FRAMES: &str =
    "the arena tried to GROW and the frame allocator had none left (frame::alloc — the FRAME bound, not the arena's)";
pub const BOUND_UNMAPPED: &str =
    "the growth region is not inside the identity map (paging::NORMAL_MAP_END) — a span the MMU cannot translate";
pub const BOUND_GROW: &str =
    "another core is already growing the arena; refused rather than raced against the frame bitmap";
/// Growth POLICY bounds (2.741). The three above are accidents — no frames, no mapping, a
/// racing grower — and every one of them is the machine saying no. These two are the KERNEL
/// saying no on a machine that would have said yes, and the distinction is the whole point:
/// an operator told "out of frames" on a box with 200 MiB free goes hunting a leak that is
/// not there, while an operator told "the heap has had its share" knows the figure to change
/// and knows the memory is still on the machine, being kept for someone else.
pub const BOUND_CAP: &str =
    "POLICY: the arena has reached its share of managed memory (heap::ARENA_SHARE_DEN) — memory is free, the heap may not have it";
pub const BOUND_RESERVE: &str =
    "POLICY: growing would take the frame pool below the floor kept for everyone else (heap::FRAME_FLOOR_DEN) — page tables and drivers allocate too";

/// The largest share of managed memory the arena may ever grow to: one part in this many.
///
/// A FRACTION and not a byte count, resolved from [`crate::frame::total_frames`] at growth
/// time, because 2.740's whole achievement was retiring the link-time ceiling — a constant
/// `MAX_ARENA` would re-introduce exactly what was removed, and would be wrong on both a
/// 64 MiB board and a 4 GiB one for opposite reasons.
pub const ARENA_SHARE_DEN: usize = 4;
/// The floor growth will not take the frame pool below: one part in this many of the frames
/// the machine manages.
///
/// The heap is the only allocator on this kernel that thinks to ask for more, so without a
/// floor it wins every race for the last frame — and what loses is not the heap but
/// `paging`'s tables, `process::build_address_space`, and the driver frames Phase 3 exists
/// to hand out. Those callers cannot grow, retry, or refuse gracefully; this is their share,
/// held back before the heap is served rather than mourned afterwards.
pub const FRAME_FLOOR_DEN: usize = 8;

/// The unit the arena is actually spent in (2.735).
///
/// One kernel stack. `sched::spawn` takes one per kernel thread and `process::spawn` one
/// per EL0 process, and at 16 KiB apiece they are the heap's largest customer by three
/// orders of magnitude — everything else the kernel allocates is a `Vec` of a few dozen
/// bytes. The figure lives HERE, not in the two callers that used to each declare their
/// own copy of it, because it is the divisor that turns the arena's remaining bytes into
/// the only number an operator can act on: how many more threads this machine can start.
pub const STACK_SIZE: usize = 16 * 1024;
/// Alignment of a kernel stack. 16 is the AArch64 SP alignment requirement — a stack
/// pointer that is not 16-byte aligned faults on the first `stp` in the prologue.
pub const STACK_ALIGN: usize = 16;

/// A request the arena can serve: where it lands and what it costs.
#[derive(PartialEq, Eq)]
pub struct Grant {
    /// Address handed to the caller (already aligned).
    pub start: usize,
    /// Arena offset after the grant — what `USED` becomes if this grant commits.
    pub new_used: usize,
    /// Bytes skipped ahead of `start` to satisfy the alignment. Charged to the arena
    /// and never handed to anyone: the difference between what callers asked for and
    /// what the heap spent on them.
    pub padding: usize,
}

/// Where a request would land in an arena of `arena` bytes at `base` with `used` already
/// handed out — the bump step as a **pure function**, the same shape (and for the same
/// reason) as `frame::plan` and `paging::window`.
///
/// The live kernel allocates a few KiB and never comes near 1 MiB, so every refusal path
/// below is code that has otherwise never run. `heap test` runs it.
///
/// Total by construction: no argument can make it overflow, wrap or panic — an absurd
/// size or a garbage alignment is refused by name.
pub fn place(
    base: usize,
    used: usize,
    arena: usize,
    size: usize,
    align: usize,
) -> Result<Grant, &'static str> {
    if align == 0 || !align.is_power_of_two() {
        return Err(BOUND_ALIGN);
    }
    // Checked first: a request bigger than the reservation is refused the same way on an
    // empty arena as on a full one, and saying so is the whole point of the distinction.
    if size > arena {
        return Err(BOUND_REQUEST);
    }
    // An arena that does not fit in the address space serves nothing: refuse it rather
    // than hand back a start address that wrapped around zero.
    if base.checked_add(arena).is_none() {
        return Err(BOUND_ARENA);
    }
    // Padding computed on the address, because it is the *address* that must be aligned,
    // and by negation rather than by rounding up, so a base near the top of the address
    // space cannot wrap a sum past it.
    let pad = base.wrapping_add(used).wrapping_neg() & (align - 1);
    let off = match used.checked_add(pad) {
        Some(v) => v,
        None => return Err(BOUND_ARENA),
    };
    let new_used = match off.checked_add(size) {
        Some(v) => v,
        None => return Err(BOUND_ARENA),
    };
    if new_used > arena {
        return Err(BOUND_ARENA);
    }
    Ok(Grant { start: base.wrapping_add(off), new_used, padding: pad })
}

/// How many FURTHER requests of `size`/`align` an arena in this state would serve —
/// [`place`] asked the only question an operator actually has: *how much room is left,
/// in the unit the room gets spent in.*
///
/// 2.734 made the bound named. A named bound is still not a legible one while it is
/// quoted in bytes: "917 KiB free" and "56 more threads" are the same fact, but only the
/// second one can be compared against a workload. Bytes also over-state the room, because
/// alignment padding is charged per grant and a byte count cannot know how many grants
/// the bytes will be cut into.
///
/// Pure and total, like `place` itself: it re-places against its own running total rather
/// than dividing, so padding is counted the same way the allocator will actually charge
/// it. `size == 0` is [`usize::MAX`] — a zero-byte request consumes nothing, so the arena
/// serves unboundedly many and the loop below would never end. Otherwise each iteration
/// adds at least one byte to a total bounded by `arena`, so it terminates.
pub fn capacity(base: usize, used: usize, arena: usize, size: usize, align: usize) -> usize {
    if size == 0 {
        return usize::MAX;
    }
    let mut at = used;
    let mut n = 0usize;
    while let Ok(g) = place(base, at, arena, size, align) {
        at = g.new_used;
        n += 1;
    }
    n
}

// --- 2.737: the arithmetic of giving memory back ------------------------------------
//
// 2.734-2.736 made exhaustion legible. None of it touched *why* the arena fills, which
// is `dealloc` below: the allocator is monotonic, so `free` only ever falls and the
// machine's ceiling is measured in uptime rather than in bytes. The fix is a free list
// threaded through the freed blocks themselves — no side table, because bookkeeping that
// needs the allocator cannot run when the allocator is what ran out.
//
// This step lands that list's *arithmetic*, pure and total, and makes the decay it is
// meant to stop a **measured** figure rather than an assertion (see [`freed`] and
// [`reclaimable`]). Threading the nodes through live memory and switching `GlobalAlloc`
// onto them is the next step; the policy below is the part that has to be right first,
// and it is the part a synthetic arena can prove.

/// What a free block must carry to be a list node: `next` and `len`, one word each.
/// A returned block smaller than this cannot hold its own bookkeeping and is the reason
/// [`reclaim_span`] can refuse — not every `free` is recoverable, and saying which are
/// is more use than a total that quietly counts bytes it can never hand out again.
pub const NODE_SIZE: usize = 2 * core::mem::size_of::<usize>();
/// Alignment a node needs. The caller's block is aligned to *its* `Layout`, which may be
/// 1 (every `Vec<u8>` in the kernel), so the node start is trimmed forward to this and
/// the skipped bytes are lost — counted, never silently absorbed.
pub const NODE_ALIGN: usize = core::mem::align_of::<usize>();

/// A returned block, trimmed to the part that can actually carry a node.
#[derive(PartialEq, Eq)]
pub struct Span {
    /// First address of the reclaimed run (node-aligned).
    pub start: usize,
    /// Bytes from `start` to the end of the returned block.
    pub len: usize,
    /// Bytes of the returned block skipped to reach `start`. Unrecoverable.
    pub lost: usize,
}

/// The span of a returned block that a free list can take back, or `None` if none of it
/// can — the block is shorter than a node, or trimming for alignment leaves it so.
///
/// Pure and total, the same shape as [`place`]: no `ptr`/`size` pair can make it wrap or
/// panic, because the failure cases are the answer rather than an edge case.
pub fn reclaim_span(ptr: usize, size: usize) -> Option<Span> {
    let end = ptr.checked_add(size)?;
    let pad = ptr.wrapping_neg() & (NODE_ALIGN - 1);
    let start = ptr.checked_add(pad)?;
    if start > end {
        return None;
    }
    let len = end - start;
    if len < NODE_SIZE {
        return None;
    }
    Some(Span { start, len, lost: pad })
}

/// How a request lands inside a free block, and what is left over on either side.
#[derive(PartialEq, Eq)]
pub struct Carve {
    /// Address handed to the caller.
    pub at: usize,
    /// The run before `at`, if it is long enough to stay a node in its own right.
    pub head: Option<(usize, usize)>,
    /// The run after the request, on the same condition.
    pub tail: Option<(usize, usize)>,
    /// Remainder bytes too short to carry a node, on either side. They stay attached to
    /// the grant and are gone until the caller frees it — bounded by `2 * NODE_SIZE` per
    /// carve, which is why splitting is safe to do greedily.
    pub lost: usize,
}

/// Cut a request of `size`/`align` out of the free block `[start, start + len)`, or
/// `None` if it does not fit — which is exactly the test a first-fit scan makes at each
/// node, so the scan is a loop over this function and nothing else.
///
/// Pure and total. Alignment is validated rather than masked, for [`place`]'s reason: a
/// hand-built request must be refused, not served a wild address.
pub fn carve(start: usize, len: usize, size: usize, align: usize) -> Option<Carve> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    let end = start.checked_add(len)?;
    let pad = start.wrapping_neg() & (align - 1);
    let at = start.checked_add(pad)?;
    let stop = at.checked_add(size)?;
    if stop > end {
        return None;
    }
    let mut lost = 0usize;
    let head = if pad >= NODE_SIZE {
        Some((start, pad))
    } else {
        lost += pad;
        None
    };
    let rest = end - stop;
    let tail = if rest >= NODE_SIZE {
        Some((stop, rest))
    } else {
        lost += rest;
        None
    };
    Some(Carve { at, head, tail, lost })
}

/// Do two free runs touch, so that one node can replace both?
///
/// The whole of coalescing, and the reason the list is kept in address order: without
/// this, a long alloc/free run leaves the arena as a heap of crumbs each too short to
/// serve anything, which is a slower version of the exhaustion the free list exists to
/// prevent. Checked-add rather than `+` so a run at the top of the address space says
/// "not adjacent" instead of wrapping into a false merge.
pub fn adjacent(a_start: usize, a_len: usize, b_start: usize) -> bool {
    a_start.checked_add(a_len) == Some(b_start)
}

// --- 2.740: the arithmetic of growing the arena --------------------------------------
//
// Two questions, and both are answered BEFORE a frame is touched, for the reason the whole
// milestone is ordered the way it is: a span the MMU does not translate is not a red test,
// it is a kernel that cannot boot. So "is the growth region mapped" is a pure predicate the
// self-test can put unmapped addresses in front of, and "how many frames does this request
// need" is arithmetic that cannot wrap, rather than a division written at the call site.

/// Does `[pa, pa + len)` lie inside the identity map that ends at `map_end`?
///
/// The gate on growth. `frame::plan` already clamps the managed range to
/// [`crate::paging::NORMAL_MAP_END`], so on this board the answer is always yes — which is
/// exactly why it is checked rather than assumed: the clamp lives in another file, and a
/// board whose map ends below its DRAM would otherwise hand the heap a span that faults on
/// the first node write. Checked-add, so a span at the top of the address space says "not
/// mapped" instead of wrapping into a false yes.
pub fn span_mapped(pa: usize, len: usize, map_end: usize) -> bool {
    match pa.checked_add(len) {
        Some(end) => end <= map_end,
        None => false,
    }
}

/// How many frames a request of `size`/`align` needs at worst, or `None` if no number of
/// frames could serve it.
///
/// Worst case on purpose: the run has to hold the request *and* the alignment pad the carve
/// may take out of its front, so the width asked for is `size + align`. Over-asking costs a
/// frame the list keeps and hands out later; under-asking costs a growth that grew and still
/// could not serve, which reads to the operator as growth not working.
pub fn grow_frames(size: usize, align: usize) -> Option<usize> {
    let want = size.checked_add(align)?;
    let f = crate::frame::FRAME_SIZE;
    Some(want.checked_add(f - 1)? / f)
}

/// Whether POLICY lets the arena take `want` more bytes right now — `None` if it may, or
/// the bound that says why not (2.741).
///
/// Pure and total on purpose, in the shape [`place`] and [`grow_frames`] established: every
/// figure it decides on is an argument, so the two limits that keep the heap from eating the
/// machine can be proved in `heap test` against synthetic pools no live board will ever
/// produce — a cap that binds, a floor that binds, and the same request answered differently
/// against a pool of a different size.
///
/// Two limits, because they fail differently. The CAP is about the arena: however much RAM
/// the machine has, one allocator does not get all of it, and the share is resolved from
/// `total_frames` so the answer scales with the board instead of with the linker. The FLOOR
/// is about everyone else: [`crate::frame::alloc`] has callers that cannot grow, retry or
/// refuse — page tables, address-space builds, driver frames — and this is the reserve those
/// callers are held to be owed *before* the heap is served.
///
/// `arena` is the whole arena, `.bss` reservation included, measured against a cap drawn
/// from managed memory that the reservation is not part of. Deliberately conservative: it
/// costs the heap its first mebibyte of headroom and cannot ever let the sum of the two
/// allocators exceed the machine.
pub fn grow_allowed(
    arena: usize,
    want: usize,
    free_frames: usize,
    total_frames: usize,
) -> Option<&'static str> {
    let f = crate::frame::FRAME_SIZE;
    // The cap, resolved from the pool as it is today rather than from a constant.
    let managed = total_frames.saturating_mul(f);
    let cap = managed / ARENA_SHARE_DEN;
    if arena.saturating_add(want) > cap {
        return Some(BOUND_CAP);
    }
    // The floor. `want` is rounded UP to the frames it will actually cost, because the pool
    // is spent in frames and a half-frame ask still takes a whole one out of the reserve.
    let need = want.saturating_add(f - 1) / f;
    let floor = total_frames / FRAME_FLOOR_DEN;
    if free_frames < need.saturating_add(floor) {
        return Some(BOUND_RESERVE);
    }
    None
}

/// How many more frames growth would ACTUALLY take, given both policy limits (2.741).
///
/// What `heap` offers the operator. The free pool alone is a promise the heap will not keep:
/// it is the answer to "how much memory is there", and the question an operator is asking
/// when they read that row is "how much more can this arena get" — which since this
/// milestone is the smaller of the room left under the cap and the frames left above the
/// floor.
pub fn grow_headroom(arena: usize, free_frames: usize, total_frames: usize) -> usize {
    let f = crate::frame::FRAME_SIZE;
    let cap = total_frames.saturating_mul(f) / ARENA_SHARE_DEN;
    let under_cap = cap.saturating_sub(arena) / f;
    let above_floor = free_frames.saturating_sub(total_frames / FRAME_FLOOR_DEN);
    if under_cap < above_floor {
        under_cap
    } else {
        above_floor
    }
}

/// A free-list node, written into the freed block itself (2.738).
///
/// `repr(C)` so the two words are at offsets 0 and 8 and nothing else: the whole reason a
/// list can exist without a side table is that this structure *is* the free memory, so its
/// layout has to be the one [`NODE_SIZE`] and [`reclaim_span`] were written against rather
/// than whatever the compiler would otherwise choose.
#[repr(C)]
struct Node {
    next: *mut Node,
    len: usize,
}

/// A first-fit, address-ordered free list whose nodes live in the blocks they describe.
///
/// 2.737 proved this policy over a fixed array ([`FreeSim`]), which was the right first
/// step and is not the same claim: an array cannot get a pointer wrong, cannot have a node
/// overwritten by the caller it just served, and cannot lose the list to a grant carved
/// out from under it. This is the same algorithm with the array removed — every decision
/// still comes from [`reclaim_span`], [`carve`] and [`adjacent`], so what is new here is
/// exactly the pointer work and nothing else.
///
/// Since 2.739 this IS the kernel's allocator: [`KernelAlloc::alloc`] scans one of these
/// over the arena before it bumps. `heap test` still drives a second, private instance over
/// [`TEST_ARENA`] — the cases that hold a whole arena full of grants at once, or run it out
/// of memory on purpose, cannot be asked of the list every `format!` in the kernel is going
/// through.
pub struct FreeList {
    head: *mut Node,
    /// Bytes the policy could not keep: alignment trim, and remainders shorter than a
    /// node. Counted rather than absorbed, so "free bytes" never overstates what can be
    /// handed out.
    lost: usize,
}

impl FreeList {
    pub const fn new() -> Self {
        FreeList { head: null_mut(), lost: 0 }
    }

    /// Return `[start, start + len)` to the list, merging it with either neighbour it
    /// touches.
    ///
    /// # Safety
    /// `[start, start + len)` must be memory this list owns, unaliased, and at least
    /// [`NODE_SIZE`] bytes aligned to [`NODE_ALIGN`] — which is precisely what
    /// [`reclaim_span`] returns, and why [`give_back`](Self::give_back) is the entry point
    /// callers should use.
    pub unsafe fn insert(&mut self, start: usize, len: usize) {
        let mut prev: *mut Node = null_mut();
        let mut cur = self.head;
        while !cur.is_null() && (cur as usize) < start {
            prev = cur;
            cur = (*cur).next;
        }
        // The write that makes the block its own bookkeeping. Everything after this is
        // ordinary list surgery; this line is the one that needs the block to be free.
        let node = start as *mut Node;
        (*node).next = cur;
        (*node).len = len;
        if prev.is_null() {
            self.head = node;
        } else {
            (*prev).next = node;
        }
        // Forward first, for `FreeSim`'s reason: absorbing the successor leaves `node`
        // still the new run, so the backward check below sees the already-grown block and
        // one insert can collapse three runs into one.
        let next = (*node).next;
        if !next.is_null() && adjacent(node as usize, (*node).len, next as usize) {
            (*node).len += (*next).len;
            (*node).next = (*next).next;
        }
        if !prev.is_null() && adjacent(prev as usize, (*prev).len, node as usize) {
            (*prev).len += (*node).len;
            (*prev).next = (*node).next;
        }
    }

    /// First fit: the first node [`carve`] accepts. Returns the granted address.
    ///
    /// # Safety
    /// The list must be well-formed (only ever built through this type's methods).
    pub unsafe fn take(&mut self, size: usize, align: usize) -> Option<usize> {
        let mut prev: *mut Node = null_mut();
        let mut cur = self.head;
        while !cur.is_null() {
            let start = cur as usize;
            let len = (*cur).len;
            let next = (*cur).next;
            if let Some(c) = carve(start, len, size, align) {
                // Unlink BEFORE writing anything back into the block. The node's own two
                // words sit at `start`, which is inside the run being carved — the head
                // remainder's node is written over them, and the grant itself may cover
                // them. Reading `next` above and dropping the node here is what makes
                // that safe; a version that re-read `(*cur).next` after the carve would
                // be reading the caller's memory.
                if prev.is_null() {
                    self.head = next;
                } else {
                    (*prev).next = next;
                }
                self.lost += c.lost;
                // Through `give_back`, NOT `insert`. `carve` guarantees each remainder is
                // long enough to hold a node; it does not guarantee either one STARTS
                // where a node can be written. The tail begins at `at + size`, so any
                // request whose size is not a multiple of `NODE_ALIGN` — 300 B, say —
                // leaves its remainder at a misaligned address, and `insert` would write
                // the list's two words there. Over a fixed array that is invisible: an
                // array entry does not care what address it holds. In real memory it is a
                // misaligned dereference, and this is the bug the milestone existed to
                // find. `reclaim_span` is the function whose entire job is "can a node
                // live at this address", so the remainders go through the same door every
                // returned block does — trimmed up to alignment, charged for the trim,
                // refused outright if that leaves them too short. The skipped bytes are
                // gone either way; the choice is whether the list SAYS so or corrupts
                // itself.
                if let Some((hs, hl)) = c.head {
                    self.give_back(hs, hl);
                }
                if let Some((ts, tl)) = c.tail {
                    self.give_back(ts, tl);
                }
                return Some(c.at);
            }
            prev = cur;
            cur = next;
        }
        None
    }

    /// Hand a block back, taking whatever part of it can carry a node.
    ///
    /// # Safety
    /// `ptr`/`size` must describe a block previously granted by this list (or memory
    /// donated to it), no longer in use by anyone.
    pub unsafe fn give_back(&mut self, ptr: usize, size: usize) {
        match reclaim_span(ptr, size) {
            Some(sp) => {
                self.lost += sp.lost;
                self.insert(sp.start, sp.len);
            }
            // Shorter than a node, or trimmed below one. Unrecoverable, and said so
            // rather than counted as free memory that can never be handed out.
            None => self.lost += size,
        }
    }

    /// Number of runs in the list. The fragmentation figure: for a given free total,
    /// fewer is strictly better, and 1 means the arena is whole.
    ///
    /// # Safety
    /// The list must be well-formed.
    pub unsafe fn nodes(&self) -> usize {
        let (mut n, mut cur) = (0usize, self.head);
        while !cur.is_null() {
            n += 1;
            cur = (*cur).next;
        }
        n
    }

    /// # Safety
    /// The list must be well-formed.
    pub unsafe fn free_bytes(&self) -> usize {
        let (mut t, mut cur) = (0usize, self.head);
        while !cur.is_null() {
            t += (*cur).len;
            cur = (*cur).next;
        }
        t
    }

    /// The largest single run, which is the only figure that says what the list can still
    /// serve: plenty of free bytes held as crumbs serves nothing.
    ///
    /// # Safety
    /// The list must be well-formed.
    pub unsafe fn largest(&self) -> usize {
        let (mut m, mut cur) = (0usize, self.head);
        while !cur.is_null() {
            if (*cur).len > m {
                m = (*cur).len;
            }
            cur = (*cur).next;
        }
        m
    }

    pub fn lost(&self) -> usize {
        self.lost
    }

    /// Is every invariant the algorithm depends on still true — strictly ascending
    /// addresses, no run overlapping the next, no two runs left adjacent (which would mean
    /// a coalesce was missed), and every run inside `[lo, hi)`?
    ///
    /// Checked after each phase of the self-test rather than only at the end: a list that
    /// merely *totals* correctly can still be corrupt, and a corrupt list under
    /// `GlobalAlloc` hands two callers the same bytes.
    ///
    /// # Safety
    /// Traverses the list; safe for any list built through this type.
    pub unsafe fn intact(&self, lo: usize, hi: usize) -> bool {
        let mut cur = self.head;
        let mut prev_end = lo;
        let mut first = true;
        while !cur.is_null() {
            let (s, l) = (cur as usize, (*cur).len);
            if s < lo || l < NODE_SIZE || s.checked_add(l).map_or(true, |e| e > hi) {
                return false;
            }
            if !first && (s < prev_end || s == prev_end) {
                // `s == prev_end` is not corruption but an un-coalesced pair, which is the
                // failure this list exists to prevent and so is treated as one.
                return false;
            }
            if s % NODE_ALIGN != 0 {
                return false;
            }
            prev_end = s + l;
            first = false;
            cur = (*cur).next;
        }
        true
    }
}

/// The kernel's own free list, and the lock that keeps two cores out of each other's
/// pointer surgery (2.739).
///
/// The list is a linked structure, so the lock-free CAS that guards the bump cursor is not
/// available to it: a scan cannot be retried against a losing snapshot the way `place` can.
/// What guards it is a **try-lock that never spins**, and that is a deliberate choice
/// rather than a cheap one:
///
/// * this kernel runs a secondary core (`kernel_secondary`) and takes timer and UART
///   interrupts on the primary, so `alloc` really can be re-entered — and a spinlock taken
///   by an interrupted allocator would be waited on **by the same core**, which is a hang,
///   not a slow path;
/// * every caller of this lock already has somewhere correct to go when it cannot have it.
///   `alloc` bumps, exactly as it did before this milestone. `dealloc` charges the block as
///   lost and says so ([`missed_frees`]), which is the truth: those bytes really are gone.
///
/// So contention costs memory and never correctness, and the failure mode of the whole
/// milestone degrades to the allocator the kernel had yesterday.
struct ListCell {
    lock: AtomicBool,
    list: UnsafeCell<FreeList>,
}
// SAFETY: the `UnsafeCell` is only ever reached through `with_list`, which hands the `&mut`
// to the one caller that won the CAS below and drops it before releasing.
unsafe impl Sync for ListCell {}

static ARENA_LIST: ListCell =
    ListCell { lock: AtomicBool::new(false), list: UnsafeCell::new(FreeList::new()) };

/// Run `f` against the kernel's free list, or `None` if the list is busy.
///
/// `f` must not allocate: it holds the lock the allocator needs, and a nested `alloc` would
/// simply be refused the list and bump instead — correct, but it would silently stop the
/// reclaiming this milestone is for. Nothing in [`FreeList`] allocates.
fn with_list<R>(f: impl FnOnce(&mut FreeList) -> R) -> Option<R> {
    if ARENA_LIST
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return None;
    }
    // SAFETY: the CAS above is the exclusion — this is the only path to the cell, and the
    // reference dies before the store that releases it.
    let r = f(unsafe { &mut *ARENA_LIST.list.get() });
    ARENA_LIST.lock.store(false, Ordering::Release);
    Some(r)
}

/// Serve a request out of reclaimed memory, or `None` if the list is busy or holds nothing
/// that fits — the reclaiming half of [`KernelAlloc::alloc`], kept out of it so the
/// accounting is stated once.
///
/// # Safety
/// The list must only ever have been built through [`FreeList::give_back`].
unsafe fn list_take(size: usize, align: usize) -> Option<(usize, usize)> {
    let (at, charged) = with_list(|l| {
        let before = l.lost();
        let at = l.take(size, align);
        (at, l.lost() - before)
    })?;
    let at = at?;
    // Conservation, and the reason the deltas are computed rather than re-traversed: the
    // run the carve came out of leaves the list, `size` of it goes to the caller, `charged`
    // of it becomes a hole, and every remaining byte is inserted straight back. So whatever
    // shape the carve took, the list's free total falls by exactly `size + charged`.
    LIST_LOST.fetch_add(charged, Ordering::Relaxed);
    LIST_FREE.fetch_sub(size + charged, Ordering::Relaxed);
    REUSED.fetch_add(1, Ordering::Relaxed);
    REUSED_BYTES.fetch_add(size, Ordering::Relaxed);
    // The charge goes back to the caller so `used` can rise by it too: those bytes were
    // free a moment ago and are now a hole, and a hole is not free memory.
    Some((at, charged))
}

/// Hand a block back to the kernel's list, and answer with the bytes it could actually
/// keep — which is what `used` falls by, and is less than `size` when the block had to be
/// trimmed to node alignment or was too short to hold a node at all.
///
/// Zero if the list was busy: the bytes are then charged as lost, because mislaying them
/// silently would leave `used` overstating the arena forever with nothing saying why.
///
/// # Safety
/// `ptr`/`size` must describe a block this allocator granted and no one still holds.
unsafe fn list_give(ptr: usize, size: usize) -> usize {
    match with_list(|l| {
        let before = l.lost();
        l.give_back(ptr, size);
        l.lost() - before
    }) {
        Some(charged) => {
            let kept = size - charged;
            LIST_LOST.fetch_add(charged, Ordering::Relaxed);
            LIST_FREE.fetch_add(kept, Ordering::Relaxed);
            kept
        }
        None => {
            MISSED.fetch_add(1, Ordering::Relaxed);
            LIST_LOST.fetch_add(size, Ordering::Relaxed);
            0
        }
    }
}

/// Why the arena cannot grow right now, or `None` if it can (2.740).
///
/// Asked before any frame is taken, because the two reasons are different accidents: a
/// machine with no frames left is out of memory, and a growth region outside the identity
/// map is a *configuration* that would fault on the first node write rather than refuse.
pub fn growth_bound() -> Option<&'static str> {
    if crate::frame::total_frames() == 0 {
        return Some(BOUND_FRAMES);
    }
    // The whole managed range, not just the next frame: `frame::alloc` may hand back any
    // frame in it, so the claim that has to hold is about the range.
    if !span_mapped(
        crate::frame::base(),
        crate::frame::end() - crate::frame::base(),
        crate::paging::NORMAL_MAP_END,
    ) {
        return Some(BOUND_UNMAPPED);
    }
    None
}

/// Hand one frame to the kernel's free list, or give it straight back to `frame::` if the
/// list is busy — a mislaid frame is memory neither allocator can ever hand out again.
///
/// # Safety
/// `pa` must be a frame `frame::alloc` just returned and nobody else holds.
unsafe fn donate(pa: usize) -> bool {
    let len = crate::frame::FRAME_SIZE;
    match with_list(|l| {
        let before = l.lost();
        l.give_back(pa, len);
        l.lost() - before
    }) {
        Some(charged) => {
            // A 4 KiB frame is `NODE_ALIGN`-aligned and far longer than a node, so
            // `reclaim_span` keeps all of it — but the charge is read back rather than
            // assumed, so a future donation of an odd span cannot overstate the arena.
            LIST_LOST.fetch_add(charged, Ordering::Relaxed);
            LIST_FREE.fetch_add(len - charged, Ordering::Relaxed);
            // A charged byte of a DONATED span is a hole exactly as a charged byte of a
            // returned block is, and `used` counts holes — otherwise `lost` would climb
            // past `used` and `held = used - lost` would saturate into a lie. Zero on this
            // board (a frame is node-aligned and far longer than a node); charged anyway,
            // because the day an odd span is donated is not the day to discover that.
            if charged > 0 {
                charge(charged);
            }
            GROWN.fetch_add(len, Ordering::Relaxed);
            GROWN_FRAMES.fetch_add(1, Ordering::Relaxed);
            true
        }
        None => {
            crate::frame::free(pa);
            false
        }
    }
}

/// Take frames from `frame::alloc()` until the list holds a run that can serve
/// `size`/`align` — the growth this milestone exists for.
///
/// Frames are donated one at a time and the list coalesces them ([`adjacent`]), so a
/// contiguous run appears without this function ever asking for one: `frame::alloc` is a
/// first-fit ascending scan, so consecutive calls are usually consecutive frames, and where
/// they are not the loop simply keeps taking until some run is long enough. That is why the
/// exit condition is the list's [`FreeList::largest`] rather than a frame count — a
/// fragmented pool costs frames, not correctness.
///
/// # Safety
/// The list must only ever have been built through [`FreeList::give_back`].
unsafe fn grow(size: usize, align: usize) -> Result<(), &'static str> {
    if GROW_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(BOUND_GROW);
    }
    let r = grow_locked(size, align);
    GROW_LOCK.store(false, Ordering::Release);
    r
}

/// # Safety
/// Caller holds [`GROW_LOCK`]; see [`grow`].
unsafe fn grow_locked(size: usize, align: usize) -> Result<(), &'static str> {
    // The mapping proof FIRST, before a frame is touched.
    if let Some(why) = growth_bound() {
        return Err(why);
    }
    let want = match size.checked_add(align) {
        Some(w) => w,
        None => return Err(BOUND_REQUEST),
    };
    let need = match grow_frames(size, align) {
        Some(n) => n,
        None => return Err(BOUND_REQUEST),
    };
    // POLICY before the pool (2.741), and against the WHOLE ask: an over-share request has
    // to refuse with the arena untouched, rather than after taking frames it would then have
    // to keep. `saturating_sub` because the two figures come from one bitmap read apart.
    let free_now =
        || crate::frame::total_frames().saturating_sub(crate::frame::used_frames());
    if let Some(why) = grow_allowed(
        arena(),
        need.saturating_mul(crate::frame::FRAME_SIZE),
        free_now(),
        crate::frame::total_frames(),
    ) {
        return Err(why);
    }
    // Bounded, so a pathologically fragmented pool costs a bounded number of frames and
    // then a named refusal, rather than draining the machine one frame at a time.
    let cap = need.saturating_mul(2).saturating_add(8);
    let largest = || with_list(|l| unsafe { l.largest() }).unwrap_or(0);
    let mut taken = 0usize;
    while taken < cap {
        if largest() >= want {
            break;
        }
        // ...and once per frame, because the exit condition is the LIST's widest run: a
        // fragmented pool costs more frames than `need`, so the cap has to bind on the frame
        // that would cross it and not only on the ask that started the loop. `arena()` has
        // risen with every donation, so this converges.
        if let Some(why) = grow_allowed(
            arena(),
            crate::frame::FRAME_SIZE,
            free_now(),
            crate::frame::total_frames(),
        ) {
            return Err(why);
        }
        let pa = match crate::frame::alloc() {
            Some(pa) => pa,
            // The bound that is NOT this arena's: the machine is out of physical frames,
            // and saying "the arena is full" here would send an operator hunting a heap
            // leak on a box that has no RAM left.
            None => return Err(BOUND_FRAMES),
        };
        // Belt and braces over `growth_bound`: the range was checked, this is the frame.
        if !span_mapped(pa, crate::frame::FRAME_SIZE, crate::paging::NORMAL_MAP_END) {
            crate::frame::free(pa);
            return Err(BOUND_UNMAPPED);
        }
        if !donate(pa) {
            // The list was busy; the frame went back to `frame::` rather than being
            // mislaid. Refusing is honest — retrying here would spin inside `alloc`.
            return Err(BOUND_GROW);
        }
        taken += 1;
    }
    if largest() >= want {
        if taken > 0 {
            GROWS.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    } else {
        Err(BOUND_ARENA)
    }
}

/// Arena bytes not available to a future request: what callers hold, plus what the policy
/// has charged as unusable ([`lost`]).
///
/// The definition is unchanged from the bump's — "used + free is the arena" is still the
/// identity `heap`, `mem` and `pmap` all report against — but the figure is no longer the
/// bump cursor, because since 2.739 it FALLS. [`bump`] is the cursor; this is the arena's
/// occupancy, and the two differ by exactly what the free list is holding.
static USED: AtomicUsize = AtomicUsize::new(0);
/// How far the bump has walked. Monotonic by construction: below it lies memory that has
/// been handed out at least once, above it memory nothing has ever touched. Reclaimed
/// blocks come back to the free list rather than to this cursor, because a bump cannot
/// un-bump out of order.
static BUMP: AtomicUsize = AtomicUsize::new(0);
/// The most `USED` has ever been. Before 2.739 this was the same figure as `USED` by
/// construction, since nothing was ever given back; now they genuinely differ, and the gap
/// between them is the memory the machine finished with and got to use again.
static HIGH: AtomicUsize = AtomicUsize::new(0);
/// Of `USED`, how much went to alignment padding rather than to a caller.
static PADDING: AtomicUsize = AtomicUsize::new(0);
/// Refusals since boot, and the reason for the last one (0 none, 1 arena, 2 request,
/// 3 alignment). A code rather than a `&str` because this is written from `alloc`.
static FAILS: AtomicUsize = AtomicUsize::new(0);
static LAST_BOUND: AtomicUsize = AtomicUsize::new(0);

/// Blocks returned to [`dealloc`] since boot, and the bytes they carried (2.737).
///
/// Both were zero-information before this milestone, because `dealloc` was a no-op with
/// no counter in it: the claim "the arena decays under uptime" was true and *unmeasured*,
/// and an unmeasured decay cannot be told from a leak-free kernel by anyone reading the
/// console. These two say how much the machine hands back, and the third says how much of
/// it a free list could actually take — which is the honest version, since a block shorter
/// than a node is returned and still unrecoverable.
static FREES: AtomicUsize = AtomicUsize::new(0);
static FREED: AtomicUsize = AtomicUsize::new(0);
static RECLAIMABLE: AtomicUsize = AtomicUsize::new(0);

/// What the kernel's free list is holding, and what it has charged as unusable (2.739).
///
/// Mirrors of [`FreeList::free_bytes`] and [`FreeList::lost`], maintained by exact deltas
/// in [`list_take`]/[`list_give`] rather than by traversing the list — a report that had to
/// walk the list would need the lock, and the whole design of that lock is that nobody ever
/// has to wait for it. `heap test` checks the mirror against a real traversal.
static LIST_FREE: AtomicUsize = AtomicUsize::new(0);
static LIST_LOST: AtomicUsize = AtomicUsize::new(0);
/// Requests served out of reclaimed memory rather than by moving the bump, and their bytes.
/// The figure that says the switch is real: a kernel whose allocator still bumped would
/// report zero here however long it ran.
static REUSED: AtomicUsize = AtomicUsize::new(0);
static REUSED_BYTES: AtomicUsize = AtomicUsize::new(0);
/// Frees that arrived while the list was locked and were charged as lost instead. Never
/// silently absorbed: this is the only way the try-lock can cost anything, so it is the
/// figure that decides whether never spinning was the right trade.
static MISSED: AtomicUsize = AtomicUsize::new(0);

/// What the arena has taken from the frame allocator (2.740): bytes, frames, and how many
/// separate growth events it took.
///
/// [`GROWN`] is the reason [`arena`] is a function rather than [`size`]: the reservation is
/// still 1 MiB of `.bss` and always will be, but the arena is that plus this. The frame
/// count is published beside it because it is the figure the OTHER allocator can be checked
/// against — a frame counted as heap arena and as free memory at the same time would read as
/// the machine having more RAM than it has.
static GROWN: AtomicUsize = AtomicUsize::new(0);
static GROWN_FRAMES: AtomicUsize = AtomicUsize::new(0);
static GROWS: AtomicUsize = AtomicUsize::new(0);
/// One grower at a time. `frame::alloc` walks a static bitmap with no lock of its own — its
/// invariant is that frames are taken during setup with IRQs masked — and growth is the
/// first caller that runs at arbitrary times. A try-lock rather than a spinlock, for
/// [`with_list`]'s reason: the loser refuses (by name, [`BOUND_GROW`]) instead of waiting
/// inside an allocator that may have interrupted the lock holder on the same core.
static GROW_LOCK: AtomicBool = AtomicBool::new(false);

fn record_fail(why: &'static str) {
    FAILS.fetch_add(1, Ordering::Relaxed);
    let code = if why == BOUND_ARENA {
        1
    } else if why == BOUND_REQUEST {
        2
    } else if why == BOUND_FRAMES {
        4
    } else if why == BOUND_UNMAPPED {
        5
    } else if why == BOUND_GROW {
        6
    } else if why == BOUND_CAP {
        7
    } else if why == BOUND_RESERVE {
        8
    } else {
        3
    };
    LAST_BOUND.store(code, Ordering::Relaxed);
}

/// Bytes charged to `USED` and, if that is a new maximum, to the high-water mark.
fn charge(bytes: usize) {
    let now = USED.fetch_add(bytes, Ordering::Relaxed) + bytes;
    HIGH.fetch_max(now, Ordering::Relaxed);
}

/// The kernel's allocator: a free list with a bump behind it (2.739).
///
/// Named for what it is rather than for how it starts. Until this milestone it was a
/// `BumpAlloc` and the name was the whole truth — every grant moved a cursor forward and
/// nothing ever came back. The bump is still in here, and still the fast path on a
/// never-freed arena, but it is now the *fallback*: what the kernel allocates from first is
/// memory it already finished with.
struct KernelAlloc;

unsafe impl GlobalAlloc for KernelAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Reclaimed memory FIRST — the line this whole sequence was building towards. A
        // zero-byte request is skipped rather than carved: `carve` would accept it at the
        // first node and hand back an address inside a run it then keeps, which is legal
        // but pointless, and the bump serves it for nothing.
        if layout.size() != 0 {
            if let Some((at, charged)) = list_take(layout.size(), layout.align()) {
                charge(layout.size() + charged);
                return at as *mut u8;
            }
        }
        // ...and the bump behind it, unchanged: for an arena nothing has freed yet this is
        // still every allocation, and for one the list cannot fit it is still the answer.
        let base = addr_of_mut!(HEAP) as usize;
        loop {
            let cur = BUMP.load(Ordering::Relaxed);
            let g = match place(base, cur, HEAP_SIZE, layout.size(), layout.align()) {
                Ok(g) => g,
                Err(why) => {
                    // 2.740: before the refusal stands, the arena tries to GROW. This is
                    // the only place growth is reached from, and it is deliberately the
                    // refusal path: while the bump can serve a request the arena is a
                    // 1 MiB `.bss` reservation and costs the frame allocator nothing.
                    // An alignment refusal is skipped — no amount of memory fixes a
                    // request that is malformed — and so is a zero-byte one, which the
                    // bump serves for nothing.
                    let why = if why == BOUND_ALIGN || layout.size() == 0 {
                        why
                    } else {
                        match grow(layout.size(), layout.align()) {
                            // Grown, and the list is the only place the new span lives:
                            // the bump runs over the reservation and only over it.
                            Ok(()) => match list_take(layout.size(), layout.align()) {
                                Some((at, charged)) => {
                                    charge(layout.size() + charged);
                                    return at as *mut u8;
                                }
                                None => why,
                            },
                            // A growth bound is a DIFFERENT bound, and replaces the
                            // arena's in the message: out of frames is not out of arena.
                            Err(g) => g,
                        }
                    };
                    // The point of 2.734: a refusal that names itself. Previously this
                    // was a bare null, and the failure surfaced at whichever caller
                    // unwrapped it with nothing saying the heap was what ran out.
                    // Safe to print from inside the allocator: `kprintln!` formats
                    // through `format_args!` straight to the UART and allocates nothing.
                    record_fail(why);
                    crate::kprintln!(
                        "[heap] REFUSED {} B (align {}) — {}; {} / {} B used",
                        layout.size(),
                        layout.align(),
                        why,
                        used(),
                        arena()
                    );
                    return null_mut();
                }
            };
            // The race two cores would lose without this: both read the same `cur` and
            // both compute the same `start`. The loser's compare_exchange fails, and it
            // re-`place`s against the winner's `new_used` instead of handing out memory
            // that is already someone else's.
            if BUMP
                .compare_exchange(cur, g.new_used, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                // The padding is charged to the arena and handed to nobody, so it is part
                // of `used` and part of `lost` at the same time — a byte the caller never
                // sees and no free will ever return.
                PADDING.fetch_add(g.padding, Ordering::Relaxed);
                charge(g.new_used - cur);
                return g.start as *mut u8;
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // 2.737's measurement, kept: it is what made the decay arguable in the first place,
        // and `freed` vs `reclaimable` is still the pair that says how much of what the
        // machine hands back is winnable at all.
        FREES.fetch_add(1, Ordering::Relaxed);
        FREED.fetch_add(layout.size(), Ordering::Relaxed);
        if let Some(s) = reclaim_span(ptr as usize, layout.size()) {
            RECLAIMABLE.fetch_add(s.len, Ordering::Relaxed);
        }
        // And 2.739's change: the bytes go back, and `used` FALLS — by exactly what the
        // list could keep. The trim it could not stays charged, because a byte inside no
        // run is not free memory however recently it was someone's. Saturating, so a
        // double free costs the arena a block rather than wrapping every ceiling computed
        // from `used` to `usize::MAX`.
        let kept = list_give(ptr as usize, layout.size());
        let _ = USED.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
            Some(u.saturating_sub(kept))
        });
    }
}

#[global_allocator]
static ALLOCATOR: KernelAlloc = KernelAlloc;

/// Bytes of heap currently handed out (for boot diagnostics).
pub fn used() -> usize {
    USED.load(Ordering::Relaxed)
}

/// The `.bss` reservation, in bytes — the arena's compile-time part, and the only part the
/// bump ever walks.
pub const fn size() -> usize {
    HEAP_SIZE
}

/// The whole arena today: the reservation plus every byte grown into it (2.740).
///
/// The figure `used + free` adds up to, and the one `heap`, `mem` and `pmap` must all
/// report — the moment the arena is a variable, a report quoting [`size`] as the total is
/// describing a second, smaller arena that no longer exists.
pub fn arena() -> usize {
    HEAP_SIZE + GROWN.load(Ordering::Relaxed)
}

/// Bytes the arena has taken from the frame allocator, and in how many frames.
pub fn grown() -> usize {
    GROWN.load(Ordering::Relaxed)
}

pub fn grown_frames() -> usize {
    GROWN_FRAMES.load(Ordering::Relaxed)
}

/// Growth events — a refusal the list could not satisfy that the frames then could.
pub fn grows() -> usize {
    GROWS.load(Ordering::Relaxed)
}

/// First address of the arena. Inside the kernel image (`.bss`), and therefore below
/// `frame::base()` — see [`describe`], which checks that rather than asserting it.
pub fn base() -> usize {
    addr_of_mut!(HEAP) as usize
}

/// First address past the arena.
pub fn end() -> usize {
    base() + HEAP_SIZE
}

/// The most the heap has ever held. Since 2.739 a real high-water mark rather than a
/// duplicate of [`used`]: the gap between the two is memory the machine finished with and
/// then got to use again.
pub fn high_water() -> usize {
    HIGH.load(Ordering::Relaxed)
}

/// How far the bump cursor has walked (2.739).
///
/// Monotonic, and no longer the same thing as [`used`]. It is the figure the bump
/// arithmetic runs against, so it is also the one [`stacks_left`] counts from: a 16 KiB
/// stack comes out of untouched arena or not at all, since the list's runs are the crumbs
/// a shell command hands back.
pub fn bump() -> usize {
    BUMP.load(Ordering::Relaxed)
}

/// Arena bytes a future request could still be served from: what the bump has never
/// touched, plus what the free list is holding. Equal to `arena() - used()` by construction
/// — [`arena`] and not [`size`] since 2.740, because grown bytes are free bytes.
pub fn free_bytes() -> usize {
    arena().saturating_sub(USED.load(Ordering::Relaxed))
}

/// Of [`used`], the part no caller holds and no free will return: alignment padding charged
/// by the bump, plus the trim and short remainders the list could not keep.
///
/// Published because it is now a figure that can grow on its own. Under a bump the only
/// loss was padding and it was bounded by the number of grants; a free list charges a hole
/// every time a carve leaves a remainder too short to be a node, and an operator watching
/// the arena is owed the difference between "in use" and "unusable".
pub fn lost() -> usize {
    PADDING.load(Ordering::Relaxed) + LIST_LOST.load(Ordering::Relaxed)
}

/// Of [`used`], the part actually in a caller's hands.
pub fn held() -> usize {
    used().saturating_sub(lost())
}

/// Bytes the free list is holding, ready to be handed out again (2.739).
pub fn list_free() -> usize {
    LIST_FREE.load(Ordering::Relaxed)
}

/// Requests served out of reclaimed memory, and their bytes. Zero on a kernel whose
/// allocator still only bumped, however long it ran — which is what makes this the figure
/// that says the switch is real.
pub fn reused() -> usize {
    REUSED.load(Ordering::Relaxed)
}

pub fn reused_bytes() -> usize {
    REUSED_BYTES.load(Ordering::Relaxed)
}

/// Frees that arrived while the list was locked and were charged as lost instead — the
/// only price the never-spinning try-lock can charge.
pub fn missed_frees() -> usize {
    MISSED.load(Ordering::Relaxed)
}

/// The live list, traversed: runs, free bytes, largest run. `None` if it is busy, which is
/// the honest answer for a report that must never make the allocator wait for it.
pub fn list_state() -> Option<(usize, usize, usize)> {
    // SAFETY: the list is only ever built through `FreeList`'s own methods.
    with_list(|l| unsafe { (l.nodes(), l.free_bytes(), l.largest()) })
}

/// Of [`used`], the part spent on alignment rather than handed to a caller.
pub fn padding() -> usize {
    PADDING.load(Ordering::Relaxed)
}

/// How many requests the heap has refused since boot.
pub fn fails() -> usize {
    FAILS.load(Ordering::Relaxed)
}

/// Blocks handed back to `dealloc` since boot (2.737).
pub fn frees() -> usize {
    FREES.load(Ordering::Relaxed)
}

/// Bytes handed back — and, today, discarded. This is the decay, in the only unit it can
/// be argued about: without it, "the shell leaks a little per command" is a guess.
pub fn freed() -> usize {
    FREED.load(Ordering::Relaxed)
}

/// Of [`freed`], the bytes a free list could actually take back. Smaller than `freed` by
/// the blocks too short to carry a node and by the alignment trim on each one, so the
/// pair states both the size of the prize and the part of it that is not winnable.
pub fn reclaimable() -> usize {
    RECLAIMABLE.load(Ordering::Relaxed)
}

/// Kernel stacks handed out by [`alloc_stack`] since boot.
static STACKS: AtomicUsize = AtomicUsize::new(0);

/// How many kernel stacks the heap has served (`heap`, `ps`).
pub fn stacks_served() -> usize {
    STACKS.load(Ordering::Relaxed)
}

/// How many MORE kernel stacks the arena can serve in its current state.
///
/// This is the arena's ceiling restated in the unit that consumes it. Before 2.735 the
/// only published figure was "1 MiB", which is true and useless: the number that decides
/// whether the next `spawn` survives is this one.
///
/// Counted from [`bump`] and not from [`used`] since 2.739, which is the conservative
/// reading and the honest one: a 16 KiB stack comes out of untouched arena or not at all,
/// because what the free list holds is the crumbs a shell command handed back. If a run
/// ever does grow long enough to serve one, this figure will *understate* the room — and a
/// spawn ceiling that promises fewer than it can serve is the failure that costs nothing.
pub fn stacks_left() -> usize {
    capacity(base(), bump(), HEAP_SIZE, STACK_SIZE, STACK_ALIGN)
}

/// Allocate one kernel stack, or refuse **by name**.
///
/// The single door for the heap's largest customer. Both spawn paths used to call
/// `alloc::alloc::alloc` themselves and answer a null with `assert!(!stack.is_null(),
/// ...)` — i.e. the arena filling up took the whole machine down, in a panic that named
/// the caller rather than the bound, instead of failing the one spawn that asked for the
/// stack that was not there. Routing both through here means the refusal carries 2.734's
/// bound up to a caller that can report it and carry on.
///
/// The reason is read back from [`last_bound`] rather than recomputed, so the string is
/// the one the allocator itself recorded for THIS refusal — spawn runs single-core with
/// IRQs masked, so nothing can slip a second refusal in between.
pub fn alloc_stack() -> Result<*mut u8, &'static str> {
    // Both arguments are compile-time constants that satisfy `Layout`'s invariants
    // (non-zero power-of-two align, size far below `isize::MAX`), so this cannot fail;
    // building it by hand avoids an `unwrap` on the spawn path.
    let layout = match Layout::from_size_align(STACK_SIZE, STACK_ALIGN) {
        Ok(l) => l,
        Err(_) => return Err(BOUND_ALIGN),
    };
    // SAFETY: non-zero size — the allocator's own contract for `alloc`.
    let p = unsafe { ALLOCATOR.alloc(layout) };
    if p.is_null() {
        return Err(last_bound().unwrap_or(BOUND_ARENA));
    }
    STACKS.fetch_add(1, Ordering::Relaxed);
    Ok(p)
}

/// The reason for the most recent refusal, or `None` if the heap has never refused one.
pub fn last_bound() -> Option<&'static str> {
    match LAST_BOUND.load(Ordering::Relaxed) {
        1 => Some(BOUND_ARENA),
        2 => Some(BOUND_REQUEST),
        3 => Some(BOUND_ALIGN),
        4 => Some(BOUND_FRAMES),
        5 => Some(BOUND_UNMAPPED),
        6 => Some(BOUND_GROW),
        7 => Some(BOUND_CAP),
        8 => Some(BOUND_RESERVE),
        _ => None,
    }
}

/// The byte count a Rust allocation failure names, if `text` is one of them.
///
/// 2.736. `alloc_stack` is the only door the kernel's *large* allocations go through, so
/// they get 2.734's named bound handed back to a caller. Everything else — every `Vec`,
/// `String` and `format!` — goes through the infallible `alloc` API, whose failure path
/// is fixed by the language: `Layout` in, null out, and the null is turned into a panic
/// by the standard allocation error handler. The kernel cannot replace that handler
/// (`#[alloc_error_handler]` is unstable; this toolchain is pinned to stable), so the
/// only place left to attribute the failure is the panic handler, and the only thing it
/// is given is the message text.
///
/// That text is `memory allocation of N bytes failed`, and it names a byte count and
/// nothing else — not which allocator refused, not how full it was, not whether the
/// machine was one allocation over the line or nowhere near it. Recovering `N` here is
/// what lets [`report_exhaustion`] answer all three from the arena's own counters.
///
/// Pure and total: a panic that is not an allocation failure returns `None` rather than
/// a misattribution, which matters because the panic handler runs on every panic.
pub fn alloc_failure_size(text: &str) -> Option<usize> {
    const HEAD: &str = "memory allocation of ";
    const TAIL: &str = " bytes failed";
    let after = match text.find(HEAD) {
        Some(i) => &text[i + HEAD.len()..],
        None => return None,
    };
    let digits = match after.find(TAIL) {
        Some(j) => &after[..j],
        None => return None,
    };
    // Rejected explicitly rather than left to `parse`: an empty run and a `+12` both
    // mean this is not the message it looks like.
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<usize>().ok()
}

/// Attribute an allocation panic to THIS arena, and state what it looked like (2.736).
///
/// Printed by the `#[panic_handler]` when [`alloc_failure_size`] recognises the message.
/// Allocates nothing — the report on a full heap must not need the heap — and reads only
/// atomics that are already loaded elsewhere on this path.
///
/// The three questions the stock message leaves open, in order: *which bound refused*
/// (from the allocator's own record of the refusal it just made), *how full the arena
/// was*, and *how close the machine was* — the last one restated in stacks, because
/// "room for 0 more" and "room for 40 more" are different situations that the same free
/// byte count can describe.
pub fn report_exhaustion(size: usize) {
    match last_bound() {
        Some(why) => crate::kprintln!("[heap] that panic is THIS arena: {} B refused — {}", size, why),
        // A null that this allocator never produced: the panic names an allocation, but
        // the heap recorded no refusal. Saying so is better than claiming the failure.
        None => crate::kprintln!(
            "[heap] a {} B allocation failed, but this arena recorded NO refusal — the \
             null did not come from here",
            size
        ),
    }
    crate::kprintln!(
        "[heap]   arena {} B at {:#010x}: {} B used ({} B of it padding), {} B free",
        arena(),
        base(),
        used(),
        padding(),
        free_bytes()
    );
    crate::kprintln!(
        "[heap]   room for {} more {} KiB stacks; {} served, {} request(s) refused since boot",
        stacks_left(),
        STACK_SIZE / 1024,
        stacks_served(),
        fails()
    );
}

/// The arena, its ceiling and where it sits relative to the frames (`heap`).
///
/// Stated whether or not the bound has fired, for 2.732's reason: a limit the operator
/// can only read once it has already bitten is indistinguishable from a crash.
pub fn describe() {
    crate::kprintln!("kernel heap: free-list arena in .bss, with a bump behind it");
    crate::kprintln!(
        "  arena          {:#010x}..{:#010x} ({} KiB, compile-time — it lives in the image)",
        base(),
        end(),
        HEAP_SIZE / 1024
    );
    // 2.740. The row above is the RESERVATION — what the linker placed, and the only part
    // the bump ever walks. This one is the rest of the arena: frames the heap has since
    // taken from `frame::` on refusals the list could not satisfy. Printed with its zeros
    // and not only once it has fired, for the reason every other bound on this screen is:
    // an operator who cannot see the figure cannot tell an arena that CAN grow from one
    // that has merely not run out yet.
    crate::kprintln!(
        "  grown          {} B in {} frame(s) over {} growth(s) — arena is {} B, not the {} B reserved",
        grown(),
        grown_frames(),
        grows(),
        arena(),
        HEAP_SIZE
    );
    // ...and whether it could grow AGAIN, asked of the other allocator rather than assumed.
    // `used_frames` is counted out of the frame bitmap itself, so "the heap took N frames"
    // is a claim checkable against the allocator those frames left.
    match growth_bound() {
        Some(why) => crate::kprintln!("  ! cannot grow  {}", why),
        None => {
            // 2.741: the headroom POLICY leaves, not the pool the machine has. The row used
            // to offer every free frame next door, which was a promise the heap would not
            // keep — an operator sizing a workload off it would be told "policy" by an
            // allocator that had just quoted them 200 MiB.
            let total = crate::frame::total_frames();
            let free = total.saturating_sub(crate::frame::used_frames());
            let head = grow_headroom(arena(), free, total);
            crate::kprintln!(
                "  can grow       {} more frame(s) ({} MiB) — its SHARE, not the pool: {} free next door, cap 1/{} of managed, floor 1/{} kept for everyone else",
                head,
                head * 4 / 1024,
                free,
                ARENA_SHARE_DEN,
                FRAME_FLOOR_DEN
            );
        }
    }
    crate::kprintln!(
        "  used           {} B ({} B of it alignment padding), {} B free",
        used(),
        padding(),
        free_bytes()
    );
    // The free bytes above, restated in the unit they get spent in (2.735). A byte count
    // is not a figure anyone can act on; "room for N more threads" is, and it is not a
    // division — each stack is re-placed against the running total, so the alignment
    // padding the arena will actually charge is counted rather than assumed away.
    crate::kprintln!(
        "  in stacks      {} served, room for {} more at {} KiB (the heap's largest customer)",
        stacks_served(),
        stacks_left(),
        STACK_SIZE / 1024
    );
    crate::kprintln!(
        "  on a refusal   the spawn fails and says which bound — not assert!/panic (2.735)"
    );
    // The other half of the same story (2.736). Stacks come through `alloc_stack` and
    // get a Result; a `Vec`/`format!` cannot, because the infallible `alloc` API has no
    // way to return one. Those still end in a panic — but no longer an anonymous one.
    crate::kprintln!(
        "  on an alloc panic  the handler recognises it, names this bound and states the arena (2.736)"
    );
    // The day named in 2.734's comment, arrived: these two were the same figure only
    // because nothing was ever given back, and the gap between them is now the memory the
    // machine finished with and got to use again.
    crate::kprintln!(
        "  high-water     {} B — used has since fallen by {} B, which a bump cannot do",
        high_water(),
        high_water().saturating_sub(used())
    );
    crate::kprintln!(
        "  bump cursor    {} B — untouched arena above it; below it, memory served at least once",
        bump()
    );
    // 2.737. The decay, measured — and 2.739, the part of it that no longer decays. `used`
    // rose forever under the bump, so the interesting number was how much of that fullness
    // was memory the machine had already finished with; now that memory comes back.
    crate::kprintln!(
        "  returned       {} B in {} block(s) — {} B of it RECLAIMED and free again",
        freed(),
        frees(),
        list_free()
    );
    crate::kprintln!(
        "  reused         {} request(s) served out of returned memory ({} B) rather than by bumping",
        reused(),
        reused_bytes()
    );
    // The counterfactual the operator can check, stated the other way round now: what the
    // arena WOULD read if the switch had not been made.
    crate::kprintln!(
        "  without it     used would read {} B instead of {} B, and would only ever rise",
        bump(),
        used()
    );
    if lost() > 0 {
        // A figure that can now grow on its own: every carve leaving a remainder too short
        // to be a node charges a hole, and a hole is neither in use nor available.
        crate::kprintln!(
            "  of used        {} B is HELD by callers; {} B is lost (padding + trim + short remainders)",
            held(),
            lost()
        );
    }
    if missed_frees() > 0 {
        crate::kprintln!(
            "  ! missed       {} free(s) arrived while the list was locked and were charged as lost",
            missed_frees()
        );
    }
    // The arena is not carved out of the frame allocator's range; it is part of the
    // image the range starts *above*. Checked here rather than claimed, because the two
    // are placed by different mechanisms (the linker and `main.rs`) and nothing else
    // would notice if they ever overlapped.
    let fbase = crate::frame::base();
    if end() <= fbase {
        crate::kprintln!(
            "  below frames   arena top {:#010x} < managed base {:#010x} — no frame overlaps it",
            end(),
            fbase
        );
    } else {
        crate::kprintln!(
            "  ! OVERLAP      arena {:#010x}..{:#010x} reaches into the frames at {:#010x}",
            base(),
            end(),
            fbase
        );
    }
    match last_bound() {
        Some(why) => crate::kprintln!("  ! refused      {} request(s) so far — last: {}", fails(), why),
        None => crate::kprintln!(
            "  on exhaustion  a NAMED refusal on the console, then a null — not a bare null"
        ),
    }
}

/// `heap grow <KiB>` — ask the live arena for more than its `.bss` reservation holds, and
/// show the growth happening (2.740).
///
/// The milestone's claim made where an operator can watch it rather than only inside
/// `heap test`: the request goes through `alloc::vec::Vec`, i.e. through the same
/// `GlobalAlloc` every `format!` in the kernel uses, and the default width is larger than
/// the whole reservation — so a served request is growth by elimination, since no state of
/// a fixed 1 MiB arena serves 1.5 MiB. The block is written end to end (a span the MMU
/// cannot translate faults here, named, rather than at whichever caller inherits it) and
/// then dropped, because growth is one-way: the frames stay the arena's, and the bytes go
/// back to the list.
pub fn grow_demo(kib: usize) {
    use alloc::vec::Vec;
    // Bounded so a typo cannot ask the machine for its whole DRAM: 16 KiB..64 MiB.
    let want = kib.clamp(16, 64 * 1024) * 1024;
    let arena0 = arena();
    let free0 = crate::frame::total_frames() - crate::frame::used_frames();
    let bump0 = bump();
    let grows0 = grows();
    crate::kprintln!(
        "[heap grow] asking for {} B ({} KiB) — the reservation is {} B, the arena is {} B",
        want,
        want / 1024,
        HEAP_SIZE,
        arena0
    );
    if want > HEAP_SIZE {
        crate::kprintln!(
            "[heap grow] that is LARGER than the whole .bss reservation: no fixed-arena state serves it"
        );
    }
    match growth_bound() {
        Some(why) => crate::kprintln!("[heap grow] cannot grow: {}", why),
        None => crate::kprintln!(
            "[heap grow] {} frame(s) free next door ({} MiB) before the ask",
            free0,
            free0 * 4 / 1024
        ),
    }
    // 2.741: POLICY is asked BEFORE the `Vec`, because the `Vec` cannot be told no. A
    // request the cap or the floor would refuse comes back from `alloc` as a null, and the
    // standard allocation error handler turns a null into a panic — so a demo that asked
    // anyway would prove the bound by killing the kernel. Asked here, the operator sees the
    // refusal by name, and can then read `heap` and `frame` back to see nothing moved.
    {
        let total = crate::frame::total_frames();
        let need = grow_frames(want, 16).unwrap_or(usize::MAX);
        if let Some(why) = grow_allowed(
            arena0,
            need.saturating_mul(crate::frame::FRAME_SIZE),
            free0,
            total,
        ) {
            crate::kprintln!("[heap grow] REFUSED before a frame was touched: {}", why);
            crate::kprintln!(
                "[heap grow] it would have cost {} frame(s); headroom is {} frame(s) of {} free — memory the machine HAS and the heap may not have",
                need,
                grow_headroom(arena0, free0, total),
                free0
            );
            crate::kprintln!(
                "[heap grow] arena INTACT at {} B ({} B grown, {} growth(s)); frames {} -> {} free",
                arena(),
                grown(),
                grows(),
                free0,
                total.saturating_sub(crate::frame::used_frames())
            );
            return;
        }
    }
    {
        let mut v: Vec<u8> = Vec::with_capacity(want);
        // `with_capacity` does not touch the memory; this does. Both ends and every byte
        // between, so an unmapped or overlapping span is a fault HERE.
        let mut i = 0usize;
        while i < want {
            v.push((i & 0xff) as u8);
            i += 1;
        }
        let at = v.as_ptr() as usize;
        let ok = v[0] == 0 && v[want - 1] == ((want - 1) & 0xff) as u8;
        crate::kprintln!(
            "[heap grow] served {} B at {:#010x} — {} (read back {} of {} B)",
            want,
            at,
            if at >= end() || at < base() { "OUTSIDE the reservation" } else { "inside the reservation" },
            if ok { "OK" } else { "MISMATCH" },
            want
        );
        crate::kprintln!(
            "[heap grow] arena {} B -> {} B ({} B grown in {} frame(s) over {} growth(s)); bump {} B -> {} B",
            arena0,
            arena(),
            grown(),
            grown_frames(),
            grows(),
            bump0,
            bump()
        );
    }
    // Dropped. The bytes are back in the list; the frames are NOT back with `frame::`.
    let free1 = crate::frame::total_frames() - crate::frame::used_frames();
    crate::kprintln!(
        "[heap grow] freed — arena STAYS {} B ({} growth(s) this ask), {} B in the list, used {} B",
        arena(),
        grows() - grows0,
        list_free(),
        used()
    );
    crate::kprintln!(
        "[heap grow] frames {} -> {} free ({} taken by the heap, and they are the heap's now)",
        free0,
        free1,
        free0.saturating_sub(free1)
    );
    crate::kprintln!(
        "[heap grow] adds up: used {} + free {} == arena {} ({}), bump {} <= reservation {} ({})",
        used(),
        free_bytes(),
        arena(),
        if used() + free_bytes() == arena() { "yes" } else { "NO" },
        bump(),
        HEAP_SIZE,
        if bump() <= HEAP_SIZE { "yes" } else { "NO" }
    );
}

/// A free list driven by [`carve`], [`reclaim_span`] and [`adjacent`] — the policy the
/// real allocator will run, over a fixed array instead of nodes threaded through live
/// memory (2.737).
///
/// The array is the *only* difference, and it is deliberate: it keeps the proof below in
/// a `heap test` that cannot corrupt the running kernel, while every decision it makes —
/// where a request lands in a block, what is left over, which neighbours merge — comes
/// from the same public functions the real list will call. Threading the nodes through
/// the blocks themselves replaces the array and nothing else.
///
/// Address-ordered, because coalescing is the whole point and neighbours are only cheap
/// to find in an ordered list.
struct FreeSim {
    spans: [(usize, usize); 32],
    n: usize,
    /// Bytes the policy could not keep — trim, and remainders shorter than a node.
    lost: usize,
    /// Insertions refused because the array was full. A real list has no such bound
    /// (its nodes live in the freed blocks); counted so the sim cannot lie by silently
    /// dropping memory and still passing.
    dropped: usize,
}

impl FreeSim {
    const fn new() -> Self {
        FreeSim { spans: [(0, 0); 32], n: 0, lost: 0, dropped: 0 }
    }

    fn remove(&mut self, i: usize) {
        let mut k = i;
        while k + 1 < self.n {
            self.spans[k] = self.spans[k + 1];
            k += 1;
        }
        self.n -= 1;
    }

    /// Insert in address order, then merge with either neighbour it touches.
    fn insert(&mut self, start: usize, len: usize) {
        if self.n == self.spans.len() {
            self.dropped += len;
            return;
        }
        let mut i = 0;
        while i < self.n && self.spans[i].0 < start {
            i += 1;
        }
        let mut j = self.n;
        while j > i {
            self.spans[j] = self.spans[j - 1];
            j -= 1;
        }
        self.spans[i] = (start, len);
        self.n += 1;
        // Forward first: merging the successor in leaves `i` still the new run's index,
        // so the backward check below sees the already-grown block and one merge can
        // collapse three runs into one.
        if i + 1 < self.n && adjacent(self.spans[i].0, self.spans[i].1, self.spans[i + 1].0) {
            self.spans[i].1 += self.spans[i + 1].1;
            self.remove(i + 1);
        }
        if i > 0 && adjacent(self.spans[i - 1].0, self.spans[i - 1].1, self.spans[i].0) {
            self.spans[i - 1].1 += self.spans[i].1;
            self.remove(i);
        }
    }

    /// First fit: the first node [`carve`] accepts.
    fn take(&mut self, size: usize, align: usize) -> Option<usize> {
        let mut i = 0;
        while i < self.n {
            let (s, l) = self.spans[i];
            if let Some(c) = carve(s, l, size, align) {
                self.remove(i);
                self.lost += c.lost;
                // Corrected in 2.738 to match `FreeList::take`, and kept identical for the
                // reason the sim exists: it is only worth running if it models the list. A
                // remainder starting at `at + size` need not be node-aligned, which an
                // array can hold happily and real memory cannot — so the sim must charge
                // the same trim, or its 1000-cycle result is a claim about a data structure
                // the kernel does not have.
                if let Some((hs, hl)) = c.head {
                    self.give_back(hs, hl);
                }
                if let Some((ts, tl)) = c.tail {
                    self.give_back(ts, tl);
                }
                return Some(c.at);
            }
            i += 1;
        }
        None
    }

    fn give_back(&mut self, ptr: usize, size: usize) {
        match reclaim_span(ptr, size) {
            Some(sp) => {
                self.lost += sp.lost;
                self.insert(sp.start, sp.len);
            }
            None => self.lost += size,
        }
    }

    fn free_bytes(&self) -> usize {
        let mut t = 0;
        let mut i = 0;
        while i < self.n {
            t += self.spans[i].1;
            i += 1;
        }
        t
    }

    /// The figure fragmentation actually shows up in: a list holding plenty of free bytes
    /// in crumbs serves nothing, and only this number says so.
    fn largest(&self) -> usize {
        let mut m = 0;
        let mut i = 0;
        while i < self.n {
            if self.spans[i].1 > m {
                m = self.spans[i].1;
            }
            i += 1;
        }
        m
    }
}

fn case_ok(pass: &mut u32, total: &mut u32, ok: bool, what: &str) {
    *total += 1;
    if ok {
        *pass += 1;
        crate::kprintln!("  [ok]   {}", what);
    } else {
        crate::kprintln!("  [FAIL] {}", what);
    }
}

/// Put [`place`] through the requests the live kernel never makes (`heap test`).
///
/// Every refusal path in this file is dead code on a machine whose whole kernel
/// allocates a few kilobytes into a mebibyte. Running the arithmetic against a synthetic
/// base/used/size is the only way "the heap refuses by name, and never hands out the
/// same bytes twice" is a claim about code that has executed.
pub fn selftest() {
    let (mut pass, mut total) = (0u32, 0u32);
    // A synthetic arena, deliberately NOT the real one: the real arena is 1 MiB and
    // empty, so its interesting boundaries are unreachable from a shell command.
    const B: usize = 0x4020_0000; // aligned base, like the real one
    const A: usize = 4096; // arena size

    crate::kprintln!("kernel heap: bump self-test against synthetic requests");

    // The ordinary case, so the failures below are read against a working baseline.
    let g = place(B, 0, A, 64, 8).unwrap();
    case_ok(&mut pass, &mut total,
        g.start == B && g.new_used == 64 && g.padding == 0,
        "an aligned request at an empty arena starts at the base and costs nothing extra");

    // Alignment padding — bytes the arena spends that no caller asked for, which is why
    // `used` and "the sum of what callers requested" are two different numbers.
    let g = place(B, 1, A, 16, 16).unwrap();
    case_ok(&mut pass, &mut total,
        g.start == B + 16 && g.padding == 15 && g.new_used == 32,
        "an unaligned bump pads FORWARD to the alignment, and the padding is charged");
    let g = place(B, 8, A, 8, 8).unwrap();
    case_ok(&mut pass, &mut total,
        g.padding == 0 && g.start == B + 8,
        "an offset already aligned pays no padding");

    // The exact fit, on both sides. The boundary is the whole reason the check is `>`
    // and not `>=`: an off-by-one here either wastes the last byte forever or hands out
    // one byte past the arena, into whatever `.bss` neighbour the linker put there.
    let g = place(B, A - 128, A, 128, 1).unwrap();
    case_ok(&mut pass, &mut total,
        g.new_used == A && g.start == B + A - 128,
        "a request that exactly fills the arena is SERVED, to the last byte");
    case_ok(&mut pass, &mut total,
        place(B, A - 128, A, 129, 1) == Err(BOUND_ARENA),
        "one byte past the exact fit is refused, and NAMED as the arena having no room");

    // Padding can be what exhausts the arena: the request fits, the alignment does not.
    // A size-only check would have handed this one out.
    case_ok(&mut pass, &mut total,
        place(B, A - 64, A, 64, 128) == Err(BOUND_ARENA),
        "a request that fits but whose ALIGNMENT does not is refused, not truncated");

    // The other bound, and the reason there are two: this one is not pressure on the
    // heap, it is a caller asking for something no arena of this size could ever serve.
    case_ok(&mut pass, &mut total,
        place(B, 0, A, A + 1, 1) == Err(BOUND_REQUEST),
        "a request larger than the WHOLE arena is a different bound, even on an empty one");
    case_ok(&mut pass, &mut total,
        place(B, 0, A, usize::MAX, 16) == Err(BOUND_REQUEST)
            && place(usize::MAX - 8, 0, A, 8, 16) == Err(BOUND_ARENA),
        "absurd sizes and a base at the top of the address space refuse, never wrap");
    case_ok(&mut pass, &mut total,
        place(B, 0, A, 8, 3) == Err(BOUND_ALIGN) && place(B, 0, A, 8, 0) == Err(BOUND_ALIGN),
        "an alignment that is not a power of two is refused by name, not masked");

    // Zero-sized allocations are legal for `Layout`, and must not be refused or
    // consume the arena.
    let g = place(B, 100, A, 0, 1).unwrap();
    case_ok(&mut pass, &mut total,
        g.new_used == 100 && g.start == B + 100,
        "a zero-byte request is served and costs the arena nothing");

    // The race. Two cores read the same `USED`, so they compute the SAME address: the
    // loser's grant overlaps the winner's. What keeps them disjoint is the CAS retry —
    // re-placing against the winner's `new_used` — so that is what is proved here, on
    // the arithmetic, rather than left to a comment above a compare_exchange.
    let a = place(B, 0, A, 96, 16).unwrap();
    let stale = place(B, 0, A, 96, 16).unwrap(); // core B, same stale `cur`
    let retry = place(B, a.new_used, A, 96, 16).unwrap(); // ...after losing the CAS
    case_ok(&mut pass, &mut total,
        stale.start == a.start && stale.start < B + a.new_used,
        "two cores off the same USED compute the SAME bytes — the race is real");
    case_ok(&mut pass, &mut total,
        retry.start >= B + a.new_used && retry.new_used > a.new_used,
        "re-placing after a lost CAS lands past the winner, so no byte is handed out twice");

    // Monotonicity, over the whole arena: bumping can never move backwards and can
    // never leave the reservation, whatever sequence of requests arrives.
    let mut used_now = 0usize;
    let mut ok = true;
    let mut served = 0u32;
    for i in 0..64usize {
        let align = 1usize << (i % 7); // 1..64
        match place(B, used_now, A, i * 3, align) {
            Ok(g) => {
                ok &= g.new_used >= used_now && g.new_used <= A && g.start >= B
                    && g.start + i * 3 <= B + A && g.start % align == 0;
                used_now = g.new_used;
                served += 1;
            }
            // Refusal must leave the arena exactly as it was — a refused request that
            // still advanced `used` would leak the arena away one failure at a time.
            Err(_) => ok &= used_now <= A,
        }
    }
    case_ok(&mut pass, &mut total, ok && served > 0 && used_now <= A,
        "over a run of mixed requests, used only ever grows and never leaves the arena");

    // --- 2.735: the room, counted in the unit that spends it -------------------------
    // `capacity` is what turns "917 KiB free" into "56 more threads". It is counted, not
    // divided, because the arena charges alignment padding per grant and a division
    // cannot see that; these cases are where the difference shows.
    case_ok(&mut pass, &mut total,
        capacity(B, 0, A, 1024, 16) == 4 && capacity(B, 0, A, A, 16) == 1,
        "an empty arena is counted in whole grants of the size that will be asked for");
    case_ok(&mut pass, &mut total,
        capacity(B, 1024, A, 1024, 16) == 3 && capacity(B, A, A, 1024, 16) == 0,
        "each grant already served takes exactly one off the count; a full arena serves 0");
    // The case a division gets wrong. 5 * 700 = 3500 <= 4096, so "bytes / size" says 5;
    // but each grant is padded up to 1024 first, so only 4 fit and the 5th is refused.
    case_ok(&mut pass, &mut total,
        capacity(B, 0, A, 700, 1024) == 4 && A / 700 == 5,
        "PADDING is counted, so the answer is smaller than free-bytes/size would claim");
    case_ok(&mut pass, &mut total,
        capacity(B, 0, A, A + 1, 16) == 0 && capacity(B, 0, A, 8, 3) == 0,
        "a request no arena state can serve counts as no room, rather than looping");
    case_ok(&mut pass, &mut total,
        capacity(B, 0, A, 0, 1) == usize::MAX,
        "a zero-byte request consumes nothing, so the room for it is unbounded, not a hang");
    // Consistency between the count and the thing counted: taking the last grant the
    // count promised must succeed, and one past it must be refused BY NAME.
    let n = capacity(B, 0, A, 384, 64);
    let mut at = 0usize;
    for _ in 0..n {
        at = place(B, at, A, 384, 64).unwrap().new_used;
    }
    case_ok(&mut pass, &mut total,
        n > 0 && place(B, at, A, 384, 64) == Err(BOUND_ARENA),
        "the count is exact: every grant it promised is served, and the next one refuses");

    // ...and the live arena, which the synthetic cases say nothing about.
    case_ok(&mut pass, &mut total,
        end() <= crate::frame::base() && base() % 16 == 0,
        "the REAL arena is 16-byte aligned and lies below the first managed frame");
    // The live ceiling in threads. It must be consistent with the bytes reported beside
    // it (it can be smaller — padding — but never larger), and it must be reachable:
    // a kernel that could not start one more thread is one spawn from the old panic.
    case_ok(&mut pass, &mut total,
        // `arena()` and a saturating subtraction since 2.740: `used` can pass the
        // reservation once the arena grows past it, and a bound that underflows is a
        // panic inside the check that exists to prevent one.
        stacks_left() > 0 && stacks_left() <= arena().saturating_sub(used()) / STACK_SIZE,
        "the live arena's room in stacks is non-zero and never over-states the free bytes");

    // --- 2.736: the panic that is really this arena ----------------------------------
    // The infallible `alloc` API cannot hand a caller a bound, so a `Vec`/`format!` that
    // the arena refuses ends in the language's own message. Recognising it is the only
    // way the panic handler can attribute the failure — and it must not MIS-attribute
    // an ordinary panic, since it runs on all of them.
    case_ok(&mut pass, &mut total,
        alloc_failure_size("memory allocation of 40 bytes failed") == Some(40)
            && alloc_failure_size(
                "panicked at 'memory allocation of 1048577 bytes failed', src/x.rs:1")
                == Some(1048577),
        "the stock allocation-failure message is recognised, alone or inside a panic line");
    case_ok(&mut pass, &mut total,
        alloc_failure_size("index out of bounds: the len is 3 but the index is 7").is_none()
            && alloc_failure_size("").is_none()
            && alloc_failure_size("memory allocation of 40 bytes").is_none(),
        "an ordinary panic is NOT attributed to the heap — a wrong name is worse than none");
    case_ok(&mut pass, &mut total,
        alloc_failure_size("memory allocation of  bytes failed").is_none()
            && alloc_failure_size("memory allocation of +8 bytes failed").is_none()
            && alloc_failure_size("memory allocation of 0 bytes failed") == Some(0),
        "a malformed count refuses rather than guessing, and a zero-byte one still parses");

    // --- 2.737: giving it back -------------------------------------------------------
    // Everything above is about a bump that never reclaims. These cases are the policy
    // that stops it being true, proved before it is allowed anywhere near live memory.

    // A returned block must be able to hold its own bookkeeping, or it is not reclaimable
    // at all. Being explicit about that is the difference between an honest figure and a
    // total that counts bytes the list could never hand out again.
    let s = reclaim_span(B, 64).unwrap();
    case_ok(&mut pass, &mut total,
        s.start == B && s.len == 64 && s.lost == 0,
        "an aligned block long enough for a node is reclaimed whole, losing nothing");
    let s = reclaim_span(B + 1, 64).unwrap();
    case_ok(&mut pass, &mut total,
        s.start == B + NODE_ALIGN && s.len == 64 - (NODE_ALIGN - 1) && s.lost == NODE_ALIGN - 1,
        "an UNALIGNED block trims forward to the node alignment, and the trim is counted");
    case_ok(&mut pass, &mut total,
        reclaim_span(B, NODE_SIZE - 1).is_none() && reclaim_span(B, NODE_SIZE).is_some(),
        "a block shorter than one node is NOT reclaimable — refused, not counted as won");
    case_ok(&mut pass, &mut total,
        reclaim_span(B + 1, NODE_SIZE).is_none() && reclaim_span(B, 0).is_none(),
        "trimming can itself make a block too short, and a zero-byte free reclaims nothing");
    case_ok(&mut pass, &mut total,
        reclaim_span(usize::MAX - 8, 64).is_none(),
        "a block that would run past the top of the address space refuses, never wraps");

    // Carving: where a request lands inside a free block, and what survives on each side.
    // A remainder shorter than a node cannot be a node, so it stays with the grant — the
    // alternative is a list entry that can never be used and can never be merged away.
    let c = carve(B, 1024, 256, 16).unwrap();
    case_ok(&mut pass, &mut total,
        c.at == B && c.head.is_none() && c.tail == Some((B + 256, 768)) && c.lost == 0,
        "a request at an aligned block starts at its base and leaves the rest as one node");
    let c = carve(B + 8, 1024, 256, 64).unwrap();
    case_ok(&mut pass, &mut total,
        c.at == B + 64 && c.head == Some((B + 8, 56)) && c.tail == Some((B + 320, 712)),
        "an alignment gap big enough to be a node is KEPT as one, not absorbed into the grant");
    let c = carve(B + 8, 1024, 256, 16).unwrap();
    case_ok(&mut pass, &mut total,
        c.at == B + 16 && c.head.is_none() && c.lost == 8,
        "a gap too short for a node is charged as loss, not left as an unusable entry");
    let c = carve(B, 256 + NODE_SIZE - 1, 256, 16).unwrap();
    case_ok(&mut pass, &mut total,
        c.tail.is_none() && c.lost == NODE_SIZE - 1,
        "a tail remainder shorter than a node is charged the same way, on the same rule");
    case_ok(&mut pass, &mut total,
        // The second one is the case a size-only scan gets wrong: 128 bytes into a
        // 128-byte block, but the block does not START aligned, so the padding pushes
        // the request past its end. `B` itself is 256-aligned, which is why the block
        // here is offset — an aligned base would have made this a false pass.
        carve(B, 128, 129, 1).is_none() && carve(B + 16, 128, 128, 256).is_none(),
        "a block too small for the request — or for its alignment — is not a fit, so the scan moves on");
    case_ok(&mut pass, &mut total,
        carve(B, 128, 8, 3).is_none() && carve(B, 128, 8, 0).is_none(),
        "a non-power-of-two alignment is refused here too, rather than masked into a wild address");

    // Adjacency, which is all coalescing is.
    case_ok(&mut pass, &mut total,
        adjacent(B, 64, B + 64) && !adjacent(B, 64, B + 65) && !adjacent(B, 64, B + 63)
            && !adjacent(usize::MAX - 63, 64, 0),
        "two runs merge only if they exactly touch, and a run at the top does not wrap into one");

    // Coalescing in the list. Three blocks freed out of order must come back as ONE run:
    // this is the case that decides whether a long alloc/free cycle keeps working or
    // slowly grinds the arena into crumbs that individually serve nothing.
    let mut sim = FreeSim::new();
    sim.give_back(B + 256, 256);
    sim.give_back(B + 768, 256); // not adjacent yet — a hole at B+512
    case_ok(&mut pass, &mut total,
        sim.n == 2 && sim.free_bytes() == 512 && sim.largest() == 256,
        "two separated frees stay two nodes — the list does not pretend a hole is not there");
    sim.give_back(B + 512, 256); // fills the hole, from BOTH sides at once
    case_ok(&mut pass, &mut total,
        sim.n == 1 && sim.spans[0] == (B + 256, 768) && sim.largest() == 768,
        "a free between two free neighbours collapses all THREE into one run, in one insert");
    case_ok(&mut pass, &mut total,
        sim.take(768, 16) == Some(B + 256) && sim.n == 0,
        "and the coalesced run then serves a request no one of its pieces could have");

    // The property the whole milestone is for. A bump allocator dies on this: `A / 512`
    // grants and then every further request is refused forever, however promptly the
    // caller frees. A reclaiming allocator does not care how long the loop runs.
    let mut bump_at = 0usize;
    let mut bump_served = 0u32;
    for _ in 0..1000 {
        match place(B, bump_at, A, 512, 16) {
            Ok(g) => {
                bump_at = g.new_used;
                bump_served += 1;
            }
            Err(_) => break,
        }
    }
    let mut sim = FreeSim::new();
    sim.insert(B, A); // the whole arena, free
    let mut cycles = 0u32;
    let mut ok = true;
    for _ in 0..1000 {
        match sim.take(512, 16) {
            Some(p) => {
                ok &= p >= B && p + 512 <= B + A && p % 16 == 0;
                sim.give_back(p, 512);
                cycles += 1;
            }
            None => break,
        }
    }
    case_ok(&mut pass, &mut total,
        bump_served == (A / 512) as u32 && bump_served < 1000,
        "the BUMP exhausts a 4 KiB arena after 8 grants, no matter that each was freed");
    case_ok(&mut pass, &mut total,
        ok && cycles == 1000 && sim.dropped == 0,
        "the free list runs the same alloc/free cycle 1000 times — 125x the arena — without exhausting it");
    case_ok(&mut pass, &mut total,
        sim.n == 1 && sim.spans[0] == (B, A) && sim.lost == 0,
        "and ends with the arena back as ONE whole run: 1000 cycles fragmented nothing");

    // Fragmentation under an interleaved pattern, which the tidy cycle above cannot show:
    // hold every block at once, then release them out of order. Coalescing is what makes
    // the arena whole again; without it these would be eight crumbs.
    let mut sim = FreeSim::new();
    sim.insert(B, A);
    let mut held = [0usize; 8];
    let mut i = 0;
    while i < 8 {
        held[i] = sim.take(512, 16).unwrap();
        i += 1;
    }
    case_ok(&mut pass, &mut total,
        sim.free_bytes() == 0 && sim.take(512, 16).is_none(),
        "a full arena refuses through the free list too — reclaiming is not overcommitting");
    for k in [1usize, 3, 5, 7, 0, 2, 4, 6].iter() {
        sim.give_back(held[*k], 512);
    }
    case_ok(&mut pass, &mut total,
        sim.n == 1 && sim.free_bytes() == A && sim.largest() == A,
        "eight blocks released out of order coalesce back to the whole arena, not eight crumbs");

    // The live figures 2.737 adds. They are counters over a `dealloc` that still discards,
    // so what is checked is that they are CONSISTENT — a reclaimable total larger than the
    // bytes returned, or a byte count with no blocks behind it, would be a false report.
    case_ok(&mut pass, &mut total,
        reclaimable() <= freed() && (frees() > 0 || freed() == 0),
        "the returned/reclaimable pair is consistent: never more won than was handed back");

    // --- 2.740: the arithmetic of GROWING it -----------------------------------------
    // Both questions the growth path asks before it touches a frame, put in front of the
    // answers no live board can produce. `span_mapped` is the one that matters: on this
    // board `frame::plan` already clamps the managed range to the identity map, so the
    // live answer is always "yes" and the refusing branch would otherwise never run — and
    // it is the branch whose failure is a fault on the first node write rather than a red
    // test.
    case_ok(&mut pass, &mut total,
        span_mapped(B, 64, B + 64) && span_mapped(B, 0, B) && !span_mapped(B, 65, B + 64),
        "a span is mapped only if it ENDS inside the map — one byte over is outside it");
    case_ok(&mut pass, &mut total,
        !span_mapped(usize::MAX - 63, 64, usize::MAX) && !span_mapped(usize::MAX, 4096, usize::MAX),
        "a span at the top of the address space says NOT mapped rather than wrapping into a false yes");

    // How wide a growth has to be. Deliberately over-asking: the run must hold the request
    // AND the pad a carve may take out of its front, because a growth that grew and still
    // could not serve reads to an operator as growth not working.
    let f = crate::frame::FRAME_SIZE;
    case_ok(&mut pass, &mut total,
        grow_frames(1, 8) == Some(1) && grow_frames(f - 8, 8) == Some(1)
            && grow_frames(f - 7, 8) == Some(2),
        "a request is rounded UP to whole frames, and the last byte over a frame costs a whole one");
    case_ok(&mut pass, &mut total,
        grow_frames(f, 8) == Some(2) && grow_frames(HEAP_SIZE, 16) == Some(HEAP_SIZE / f + 1),
        "the alignment pad is asked for as well, so a grown run can still be carved to alignment");
    case_ok(&mut pass, &mut total,
        grow_frames(usize::MAX, 16).is_none() && grow_frames(usize::MAX - 8, 8).is_none(),
        "a width that cannot be counted in frames refuses rather than wrapping to a small one");

    // The bounds are four DIFFERENT strings and `last_bound` round-trips each: an operator
    // told "the arena is full" when the machine is out of frames goes hunting a heap leak.
    case_ok(&mut pass, &mut total,
        BOUND_FRAMES != BOUND_ARENA && BOUND_UNMAPPED != BOUND_ARENA && BOUND_GROW != BOUND_ARENA
            && BOUND_FRAMES != BOUND_UNMAPPED && BOUND_UNMAPPED != BOUND_GROW,
        "a growth bound names a DIFFERENT allocator than the arena bound, and says so distinctly");

    // --- 2.741: growth's SHARE -------------------------------------------------------
    // Everything above bounds growth against accidents. These bound it against itself: the
    // heap is the only allocator here that asks for more, so without a share it wins every
    // race for the last frame and what dies is page tables and driver frames, not the heap.
    // Synthetic pools, because the live board has one size and the point is that the answer
    // is resolved from whatever size the board has.
    const P: usize = 1024; // a 4 MiB pool: cap 1 MiB (1/4), floor 128 frames (1/8)
    let pcap = P * f / ARENA_SHARE_DEN;
    let pfloor = P / FRAME_FLOOR_DEN;
    case_ok(&mut pass, &mut total,
        grow_allowed(512 * 1024, 64 * 1024, P, P).is_none() && pcap == 1024 * 1024 && pfloor == 128,
        "an ask well inside both the share and the floor is ALLOWED, and costs no bound");
    // The cap, on both sides of it. Room for exactly one more frame under the share is
    // served; the frame that would cross it is refused, on a pool with 1024 frames free.
    case_ok(&mut pass, &mut total,
        grow_allowed(pcap - f, f, P, P).is_none()
            && grow_allowed(pcap - f, 2 * f, P, P) == Some(BOUND_CAP),
        "the last frame under the share is served and the one that crosses it is refused BY POLICY");
    // The floor, on both sides of it. Nothing to do with the arena's size: this is the
    // reserve held for the callers that cannot grow, retry or refuse.
    case_ok(&mut pass, &mut total,
        grow_allowed(0, f, pfloor + 1, P).is_none()
            && grow_allowed(0, f, pfloor, P) == Some(BOUND_RESERVE),
        "growth stops at the frame floor kept for page tables and drivers, and NAMES the floor");
    // The one that says the cap is a fraction and not a constant: the SAME ask, refused on
    // a small machine and allowed on a big one, with no rebuild in between.
    case_ok(&mut pass, &mut total,
        grow_allowed(1024 * 1024, f, P, P) == Some(BOUND_CAP)
            && grow_allowed(1024 * 1024, f, 4 * P, 4 * P).is_none(),
        "the share is resolved against the pool as it is, so a bigger machine grants the same ask");
    // Total, like every other bound in this file: absurd figures refuse rather than wrap
    // into a permission. A pool of zero frames has a cap of zero, so nothing is allowed.
    case_ok(&mut pass, &mut total,
        grow_allowed(usize::MAX, f, 0, 0) == Some(BOUND_CAP)
            && grow_allowed(0, usize::MAX, usize::MAX, usize::MAX) == Some(BOUND_CAP),
        "absurd arenas, asks and pools refuse by policy rather than overflowing into a yes");
    // Policy is not out-of-memory, and the operator is owed the difference: a cap refusal on
    // a machine with 200 MiB free sends anyone reading it hunting a heap leak otherwise.
    case_ok(&mut pass, &mut total,
        BOUND_CAP != BOUND_RESERVE && BOUND_CAP != BOUND_FRAMES && BOUND_RESERVE != BOUND_FRAMES
            && BOUND_CAP != BOUND_ARENA && BOUND_RESERVE != BOUND_ARENA,
        "a POLICY refusal is a different string from out-of-frames and from arena-full");
    // And the headroom `heap` offers is the headroom growth will actually take: the smaller
    // of the two limits, exactly — one more frame than it offers is refused, by whichever
    // limit is the binding one.
    let h_cap = grow_headroom(512 * 1024, P, P);
    let h_floor = grow_headroom(0, pfloor + 2, P);
    case_ok(&mut pass, &mut total,
        h_cap == 128 && h_floor == 2 && grow_headroom(0, P, P) == pcap / f,
        "the offered headroom is the SMALLER of the share left and the frames above the floor");
    case_ok(&mut pass, &mut total,
        grow_allowed(512 * 1024, h_cap * f, P, P).is_none()
            && grow_allowed(512 * 1024, (h_cap + 1) * f, P, P) == Some(BOUND_CAP)
            && grow_allowed(0, h_floor * f, pfloor + 2, P).is_none()
            && grow_allowed(0, (h_floor + 1) * f, pfloor + 2, P) == Some(BOUND_RESERVE),
        "an ask of exactly the offered headroom is served, and one frame more is refused — the row is a promise");

    // --- 2.738: the same list, in real memory ----------------------------------------
    // Everything above ran over a fixed array, which cannot get a pointer wrong, cannot
    // have a node overwritten by the caller it just served, and cannot lose the list to a
    // grant carved out from under it. Below is the same algorithm with the array removed,
    // over a real 4 KiB arena — the last rehearsal before this list is what every
    // allocation in the kernel goes through.
    const TA_SIZE: usize = TEST_ARENA_SIZE;
    let fl_lost: usize;
    let fl_cycles: u32;
    unsafe {
        let ta = addr_of_mut!(TEST_ARENA) as usize + TEST_ARENA_SKEW;
        let mut list = FreeList::new();
        list.insert(ta, TA_SIZE);
        // The claim that distinguishes this from the sim: there is no side table. The
        // list's two words are readable at the front of the free block itself.
        let node_next = *(ta as *const usize);
        let node_len = *((ta + core::mem::size_of::<usize>()) as *const usize);
        case_ok(&mut pass, &mut total,
            list.nodes() == 1 && list.free_bytes() == TA_SIZE && list.largest() == TA_SIZE
                && node_next == 0 && node_len == TA_SIZE,
            "the list's bookkeeping is IN the freed block — no side table, so it cannot itself need the allocator");
        case_ok(&mut pass, &mut total,
            list.intact(ta, ta + TA_SIZE),
            "a one-run list over the whole arena satisfies every invariant the algorithm assumes");

        // The property, now against pointers. Each grant is written through before it is
        // returned: if a node were left inside a live grant, the write would eat it and
        // the next `take` would walk garbage — which is exactly the class of bug an array
        // cannot have and this milestone exists to rule out.
        let mut cycles = 0u32;
        let mut ok = true;
        let mut i = 0u32;
        while i < 1000 {
            match list.take(512, 16) {
                Some(p) => {
                    core::ptr::write_bytes(p as *mut u8, (i & 0xff) as u8, 512);
                    ok &= p >= ta && p + 512 <= ta + TA_SIZE && p % 16 == 0;
                    list.give_back(p, 512);
                    cycles += 1;
                }
                None => break,
            }
            i += 1;
        }
        case_ok(&mut pass, &mut total,
            ok && cycles == 1000,
            "the REAL list runs 1000 alloc/free cycles through a 4 KiB arena — 125x its size — writing over every grant");
        case_ok(&mut pass, &mut total,
            list.nodes() == 1 && list.free_bytes() == TA_SIZE && list.lost() == 0
                && list.intact(ta, ta + TA_SIZE),
            "and the arena is back as ONE whole run with nothing lost: the nodes survived every write");

        // Eight blocks held at once. The sim could only show that the totals add up; here
        // the blocks are real, so the stronger claim can be made — no two grants overlap,
        // and no node was ever placed inside one.
        let mut held = [0usize; 8];
        let mut k = 0usize;
        while k < 8 {
            held[k] = list.take(512, 16).unwrap();
            core::ptr::write_bytes(held[k] as *mut u8, (0xA0 + k) as u8, 512);
            k += 1;
        }
        let mut distinct = true;
        k = 0;
        while k < 8 {
            let mut b = 0usize;
            while b < 512 {
                distinct &= *((held[k] + b) as *const u8) == (0xA0 + k) as u8;
                b += 1;
            }
            k += 1;
        }
        case_ok(&mut pass, &mut total,
            distinct,
            "eight simultaneous grants each still hold their own bytes — no two overlap, and no node sits inside one");
        case_ok(&mut pass, &mut total,
            list.nodes() == 0 && list.take(512, 16).is_none() && list.take(8, 1).is_none(),
            "a fully granted arena refuses through the list too — reclaiming is not overcommitting");
        for k in [5usize, 1, 7, 3, 0, 6, 2, 4].iter() {
            list.give_back(held[*k], 512);
        }
        case_ok(&mut pass, &mut total,
            list.nodes() == 1 && list.free_bytes() == TA_SIZE && list.largest() == TA_SIZE
                && list.intact(ta, ta + TA_SIZE),
            "released out of order, the eight coalesce back to the whole arena through real pointers");

        // Mixed sizes and alignments, the shape a kernel actually allocates in: the tidy
        // uniform cycle above is the easy case, and a first-fit list only earns trust if
        // an irregular pattern also leaves the arena whole.
        let mut mixed = [0usize; 6];
        let sizes = [24usize, 512, 40, 128, 96, 300];
        let aligns = [8usize, 16, 8, 64, 8, 16];
        k = 0;
        while k < 6 {
            mixed[k] = list.take(sizes[k], aligns[k]).unwrap();
            ok &= mixed[k] % aligns[k] == 0;
            core::ptr::write_bytes(mixed[k] as *mut u8, (0x30 + k) as u8, sizes[k]);
            k += 1;
        }
        case_ok(&mut pass, &mut total,
            ok && list.intact(ta, ta + TA_SIZE),
            "six grants of mixed size and alignment are each honoured, and the list stays well-formed");
        // The case the array could not have: 300 is not a multiple of `NODE_ALIGN`, so
        // `carve`'s tail begins 4 B off the alignment a node needs. An array entry holds
        // that address without complaint; a node written there is a misaligned dereference
        // — which is what this ran into the first time, and why every remainder now goes
        // back through `reclaim_span`. Stated as its own case so the trap keeps a name.
        case_ok(&mut pass, &mut total,
            sizes[5] % NODE_ALIGN != 0 && list.intact(ta, ta + TA_SIZE),
            "a request whose SIZE is not node-aligned leaves its remainder trimmed and charged, never a misaligned node");
        for k in [2usize, 0, 4, 1, 5, 3].iter() {
            list.give_back(mixed[*k], sizes[*k]);
        }
        // Not one run this time, and that is the honest result — stated exactly, because
        // the exact number is the point. Two of the six carves charged a loss: the 512 B
        // grant landed in a block starting 8 B short of its 16-byte alignment, and 300 is
        // not a multiple of `NODE_ALIGN`, so its tail began 4 B off one. A charged byte is
        // a HOLE, and no amount of coalescing closes a hole — which is precisely why it is
        // charged instead of absorbed into a run the list would then hand out as if it
        // were whole. Two holes cut the arena into three runs. What must hold is that
        // every byte is in a run or in the charge: `free_bytes + lost` is the arena.
        // "Always back to one run" is a property of a loss-free workload, and 2.737's
        // uniform cycle above is one; this pattern is not, by construction.
        case_ok(&mut pass, &mut total,
            list.free_bytes() + list.lost() == TA_SIZE && list.lost() == 8 + 4
                && list.nodes() == 3 && list.intact(ta, ta + TA_SIZE),
            "an irregular pattern released out of order returns every byte not CHARGED as loss — two charges, so three runs");
        // So the largest run is the arena PAST the last hole, not the whole arena. The
        // claim that survives the irregular pattern is the one that was ever really about
        // the bump: a single run still longer than everything the six grants asked for PUT
        // TOGETHER, which really serves a request that size — where the bump had been dead
        // since its eighth grant, a thousand cycles ago.
        let big = list.largest();
        let mut asked = 0usize;
        k = 0;
        while k < 6 {
            asked += sizes[k];
            k += 1;
        }
        case_ok(&mut pass, &mut total,
            big > asked && big > TA_SIZE / 2 && list.take(big, NODE_ALIGN).is_some(),
            "the largest run still exceeds all six grants put together, and really serves a request that size");
        fl_lost = list.lost();
        fl_cycles = cycles;
    }

    // --- 2.739: the kernel's OWN arena, live -----------------------------------------
    // Everything above ran somewhere a mistake could not hurt anyone: synthetic numbers,
    // a fixed array, a private 4 KiB arena. These cases are the switch itself. The arena
    // read below is the one every `format!` on the boot path has already gone through,
    // and the figures are its own — which is why they are checked for CONSISTENCY first
    // and only then driven.

    // The identity `heap`, `mem` and `pmap` all report against, restated for an allocator
    // whose `used` can now fall. `used + free` is still the arena; and because a reclaimed
    // block goes back to the list rather than to the cursor, `used + list` is exactly how
    // far the bump has walked. Two independent counters and one derived figure agreeing is
    // what makes the whole set worth printing.
    case_ok(&mut pass, &mut total,
        // `arena()` on the left and `size()` on the bump: the sum is over the WHOLE arena
        // (2.740 made that a variable), while the bump is still bounded by the reservation
        // and always will be — grown bytes reach the kernel only through the list.
        used() + free_bytes() == arena() && used() + list_free() == bump() + grown()
            && bump() <= size() && high_water() >= used(),
        "the live arena adds up: used + free is the arena, used + list is the bump, and high-water leads used");
    case_ok(&mut pass, &mut total,
        // Path-qualified: a local `held` array above shadows the accessor by that name.
        self::held() + lost() == used() && lost() >= padding(),
        "and used splits exactly into what callers hold and what the arena charged as unusable");

    // The counters against a real traversal of the live list. They are maintained by
    // deltas precisely so no report ever has to take the lock and walk it — which is worth
    // nothing if the deltas have drifted, so here the walk happens once and they are
    // compared. `largest <= walked` is the cheap sanity that the runs are runs.
    let (nodes, walked, largest) = match list_state() {
        Some(t) => t,
        None => (usize::MAX, usize::MAX, usize::MAX),
    };
    case_ok(&mut pass, &mut total,
        walked == list_free() && largest <= walked && (nodes > 0 || walked == 0),
        "the free-list counters match a REAL traversal of the live list — the report is not a story about a second list");

    // The switch, driven. A grant raises `used`; returning it LOWERS `used` — the one
    // thing a bump allocator can never do, and the reason 2.734's `free` and 2.735's
    // `stacks_left()` stop decaying under nothing but uptime.
    let live_ok;
    let cycled_ok;
    unsafe {
        // Node-aligned on purpose. `reclaim_span` trims a returned block forward to
        // `NODE_ALIGN` and charges the trim, so a request at that alignment is the one
        // whose block comes back WHOLE — which is what makes the cycle below loss-free
        // and its "the cursor did not move" claim exact rather than approximate.
        let l = match Layout::from_size_align(640, NODE_ALIGN) {
            Ok(l) => l,
            Err(_) => Layout::new::<usize>(),
        };
        let u0 = used();
        let p = ALLOCATOR.alloc(l);
        let u1 = used();
        ALLOCATOR.dealloc(p, l);
        let u2 = used();
        live_ok = !p.is_null() && p as usize >= base() && p as usize + 640 <= end()
            && u1 >= u0 + 640 && u2 < u1;

        // ...and it keeps working. 200 cycles through the kernel's own arena, each writing
        // over its grant, must not move the cursor one byte: that is reclaiming, as opposed
        // to an arena that happens to be big enough not to have failed yet.
        let b0 = bump();
        let r0 = reused();
        let mut ok = true;
        let mut i = 0u32;
        while i < 200 {
            let q = ALLOCATOR.alloc(l);
            ok &= !q.is_null();
            if q.is_null() {
                break;
            }
            core::ptr::write_bytes(q, (i & 0xff) as u8, 640);
            ALLOCATOR.dealloc(q, l);
            i += 1;
        }
        cycled_ok = ok && bump() == b0 && reused() >= r0 + 200;
    }
    case_ok(&mut pass, &mut total, live_ok,
        "a grant from the LIVE arena raises used, and returning it LOWERS used — what a bump can never do");
    case_ok(&mut pass, &mut total, cycled_ok,
        "200 alloc/free cycles through the kernel's own arena move the bump cursor NOT ONE BYTE — every one served from returned memory");

    // The claim that actually matters, made through ordinary kernel code rather than
    // through the allocator's own back door: this is the allocator behind every `Vec`,
    // `String` and `format!` in the kernel, so a `Vec` dropped here must be the next
    // `Vec`'s memory. 88 u64s is 704 B at 8-byte alignment — node-aligned, so the block
    // comes back whole and the identical request that follows really can be served by it.
    //
    // 2.744: this check used to span the SETUP allocation as well as the claim — it took
    // `bump()` before the FIRST `Vec` and required the cursor to be unmoved at the end,
    // which silently also asserts that the first `Vec` was served from the list. That is a
    // fact about whatever ran BEFORE `heap test`, not about reclaiming, and it is why
    // `stack-room` was red at 79/80 on the same commit where `heap` and `freelist-live`
    // were 80/80. Measured on both boots rather than reasoned about — the list holds the
    // same 752 free bytes either way, but:
    //   * `stack-room` (a boot that has run an EL0 process): nodes=3, largest=640 B. A
    //     704 B request cannot be served out of a 640 B run, so it correctly fell to the
    //     bump, the cursor moved, and the old `bump() == b0` failed.
    //   * `heap` (a boot that has not): nodes=2, largest=704 B — an exact fit, served from
    //     the list, cursor unmoved, old check green.
    // First-fit refusing a 640 B block for a 704 B request is the allocator being RIGHT, so
    // the defect was in the check and not under it. The fix is therefore not to relax the
    // assertion — it is to measure across the CLAIM instead of across the setup, and to say
    // MORE than the old check said while doing it: after the drop, the next request of that
    // shape must move the cursor NOT ONE BYTE, must be served by EXACTLY one carve out of
    // the list, and must take its bytes OUT of the list's free total. That holds on both
    // boots for the same reason rather than by luck: the dropped 704 B coalesces with the
    // 640 B block beside it (measured: largest 640 -> 1344 across the drop), so the run is
    // there to serve the reclaim whatever the boot did beforehand.
    let vec_ok;
    {
        use alloc::vec::Vec;
        const SHAPE: usize = 88 * core::mem::size_of::<u64>(); // 704 B, node-aligned
        let held_used;
        {
            let mut v: Vec<u64> = Vec::with_capacity(88);
            v.push(0x5a5a_5a5a);
            held_used = used();
        }
        // The drop is the free, and `used` falls on it — for a `Vec` nobody told about
        // any of this.
        let freed_used = used();
        // Everything the claim is about is measured from HERE — after the block being
        // reclaimed is in the list, and after any state the setup happened to inherit.
        let b1 = bump();
        let r1 = reused();
        let lf1 = list_free();
        let mut v: Vec<u64> = Vec::with_capacity(88);
        v.push(0xa5a5_a5a5);
        vec_ok = freed_used < held_used            // the drop really freed, live
            && v[0] == 0xa5a5_a5a5                 // and the memory it came back as works
            && bump() == b1                        // the cursor did not move: not a bump
            && reused() == r1 + 1                  // exactly one carve out of the list
            && lf1 >= list_free() + SHAPE          // and the list is that much emptier
            && used() > freed_used;                // charged to the caller again
    }
    case_ok(&mut pass, &mut total, vec_ok,
        "a Vec dropped by ORDINARY kernel code frees into the list, and the next Vec of that shape is served out of it");

    // --- 2.740: the arena GROWS, live -------------------------------------------------
    // The claim no arithmetic above can make, and the one the milestone exists for: a
    // request LARGER THAN THE WHOLE 1 MiB RESERVATION is served. `place` refuses it as
    // BOUND_REQUEST — no state of a fixed arena could ever serve it — so the only way this
    // Vec exists is that the heap went next door to `frame::alloc` for the memory and the
    // list handed it out. Driven through `alloc::vec::Vec`, i.e. through the same
    // `GlobalAlloc` every `format!` on the boot path uses, not through a back door.
    let grow_ok;
    let big_at;
    {
        use alloc::vec::Vec;
        const BIG: usize = HEAP_SIZE + HEAP_SIZE / 2; // 1.5 MiB into a 1 MiB reservation
        let b0 = bump();
        let uf0 = crate::frame::used_frames();
        let gf0 = grown_frames();
        let mut v: Vec<u8> = Vec::with_capacity(BIG);
        // Write to both ends: an address the MMU cannot translate faults HERE, in a test
        // that names the span, rather than at whichever `format!` inherits it later.
        v.push(0x5a);
        while v.len() < BIG {
            v.push((v.len() & 0xff) as u8);
        }
        big_at = v.as_ptr() as usize;
        grow_ok = v[0] == 0x5a
            && v[BIG - 1] == ((BIG - 1) & 0xff) as u8
            && BIG > size()                    // no 1 MiB arena serves this, in any state
            && arena() > size()                // ...and the arena is demonstrably bigger
            && arena() == size() + grown()     // by exactly what it took, and no more
            && grown() == grown_frames() * crate::frame::FRAME_SIZE
            && grows() > 0
            // Not double-counted: every frame the heap took left the frame allocator,
            // so the two allocators' books move by the SAME number. Zero on a second
            // `heap test` run, where the list already holds the run and no growth is
            // needed — which is the point of checking the DELTA and not the total.
            && crate::frame::used_frames() - uf0 == grown_frames() - gf0
            // The bump never walks into grown memory: it runs over the reservation and
            // only over it, so grown bytes reach the kernel through the list alone.
            && bump() == b0
            && bump() <= size();
    }
    case_ok(&mut pass, &mut total, grow_ok,
        "a 1.5 MiB request — larger than the WHOLE .bss reservation — is served out of frames the arena grew into");
    // And the grown span is where it claims to be: outside the reservation, inside the
    // frame allocator's managed range. A grown address that fell inside `.bss` would mean
    // the growth path served out of the bump after all.
    case_ok(&mut pass, &mut total,
        (big_at >= end() || big_at < base())
            && big_at >= crate::frame::base() && big_at < crate::frame::end()
            && span_mapped(big_at, HEAP_SIZE + HEAP_SIZE / 2, crate::paging::NORMAL_MAP_END),
        "the memory it was served from is a FRAME, outside the reservation and inside the identity map");
    // The Vec above is dropped, so the 1.5 MiB is back in the list — and stays in the
    // arena. Growth is one-way by design: the frames are the heap's now.
    case_ok(&mut pass, &mut total,
        growth_bound().is_none() && list_free() >= HEAP_SIZE
            && crate::frame::used_frames() <= crate::frame::total_frames(),
        "the grown bytes come back to the LIST when freed, the arena keeps them, and the machine can still grow again");

    // And after all of that, the arena still adds up. Stated a second time on purpose: the
    // first was a snapshot of a machine that had only ever bumped-and-freed on the boot
    // path, this one follows 200 deliberate cycles, a live carve and a growth.
    case_ok(&mut pass, &mut total,
        // `arena()` and `bump() + grown()`: the sum is over the whole arena, which 2.740
        // made a variable, and every grown byte reaches the kernel through the list, so it
        // sits on the bump's side of the second identity rather than inside the cursor.
        used() + free_bytes() == arena() && used() + list_free() == bump() + grown()
            && bump() <= size()
            && match list_state() { Some((_, w, _)) => w == list_free(), None => false },
        "after every live cycle above, the GROWN arena STILL adds up and the counters still match the walk");

    crate::kprintln!(
        "kernel heap: {}/{} bump checks held (arena {} KiB at {:#010x} — {} KiB reserved + {} KiB grown, {} B used, {} refusals)",
        pass, total, arena() / 1024, base(), HEAP_SIZE / 1024, grown() / 1024, used(), fails()
    );
    crate::kprintln!(
        "kernel heap: {} B returned in {} block(s) so far, {} B of it reclaimable — and now RECLAIMED",
        freed(), frees(), reclaimable()
    );
    crate::kprintln!(
        "kernel heap: free list proved over {} B of REAL memory — {} alloc/free cycles, \
         nodes threaded through the blocks, {} B charged as loss; GlobalAlloc now reclaims",
        TEST_ARENA_SIZE, fl_cycles, fl_lost
    );
    crate::kprintln!(
        "kernel heap: LIVE arena — {} request(s) served from returned memory ({} B), \
         bump cursor at {} B while used is {} B, {} B in the list, {} B lost, {} missed free(s)",
        reused(), reused_bytes(), bump(), used(), list_free(), lost(), missed_frees()
    );
}
