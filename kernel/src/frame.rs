//! Physical frame allocator — a simple, correct bitmap over DRAM.
//!
//! Manages 4 KiB physical frames from the end of the kernel image
//! (`__kernel_end`) up to the top of RAM. One bit per frame: 0 = free,
//! 1 = allocated. First-fit. This is deliberately unsophisticated; a buddy
//! allocator can replace it later without changing the interface.
//!
//! **Where the range comes from** (milestone 2.731). Until now `main.rs` sized this
//! allocator from a raw `fdt::ram_end_or()` read of the device tree, which meant the
//! largest consumer on the board walked around every rule `board::resolved()` spent
//! milestones 2.726–2.730 establishing: a fixture tree, another machine's tree and an
//! impossible `/memory` claim would each have sized our memory. The range now arrives
//! already resolved, so the rules bound the allocator too.
//!
//! **Why a bound must speak.** Two limits shrink whatever range we are given: the
//! identity map ([`crate::paging::NORMAL_MAP_END`]) and this file's static bitmap.
//! Both are legitimate — a frame past the identity map is a translation fault, not
//! slow memory — but both used to apply in silence, so a `-m 384M` boot reported
//! "256 MiB" and looked exactly like a machine that has 256 MiB. A bound the operator
//! cannot see is indistinguishable from memory that is not there, so each one is
//! recorded with the count it cost ([`bound`], [`unmanaged_frames`]).
//!
//! **What the bitmap is sized by** (milestone 2.732). Making that second bound visible
//! showed what it was: a flat 1024 words — 65 536 frames, 256 MiB — chosen before the
//! board maps existed and unaware of them. It fits `board-virt`'s declared DRAM
//! exactly, so on the machine we boot it never binds; on `board-rpi4` (1 GiB declared)
//! or `board-pine64` (2 GiB) it would have handed most of the board's own nominal DRAM
//! back before a single live tree was read, with the map and the allocator disagreeing
//! at compile time. The bitmap is now sized from [`crate::board::ACTIVE`] and clamped by
//! the identity map, so it covers what this board says it has and no more — bits for
//! frames the MMU cannot translate would be BSS spent on addresses we may never hand
//! out. The bound then means the one thing it should: **a live tree offered more DRAM
//! than this board declares.**

use core::ptr::addr_of_mut;

pub const FRAME_SIZE: usize = 4096;

/// Frames between this board's DRAM base and the top of the identity map — every frame
/// the MMU could translate as Normal memory, whatever the tree says exists. Zero if the
/// map ends at or below the DRAM base, which would be a `paging.rs`/`board.rs`
/// disagreement rather than a small machine.
const WINDOW_FRAMES: usize = {
    let base = crate::board::ACTIVE.ram_base as usize;
    if crate::paging::NORMAL_MAP_END > base {
        (crate::paging::NORMAL_MAP_END - base) / FRAME_SIZE
    } else {
        0
    }
};

/// Frames in the DRAM this board's map *declares* (the docs/15 table). The nominal
/// figure, deliberately the smallest SKU: a live tree is expected to correct it upward,
/// and that correction is what the bitmap bound now reports.
const DECLARED_FRAMES: usize = (crate::board::ACTIVE.ram_size as usize) / FRAME_SIZE;

/// Bitmap capacity: this board's declared DRAM, clamped to what the identity map can
/// translate. On `board-virt` the clamp is slack (256 MiB declared, a 1 GiB window) and
/// this is the same 65 536 frames the constant used to hard-code; on `board-pine64` the
/// clamp binds (2 GiB declared, 1 GiB window) and stops us reserving bits for a gibibyte
/// of untranslatable addresses.
pub const MAX_FRAMES: usize = if DECLARED_FRAMES < WINDOW_FRAMES {
    DECLARED_FRAMES
} else {
    WINDOW_FRAMES
};

/// Bytes the bitmap can cover — [`MAX_FRAMES`] as a size, for reporting.
pub const BITMAP_CAPACITY: usize = MAX_FRAMES * FRAME_SIZE;

/// Never a zero-length array: a map declaring no DRAM would still be indexed by code
/// that cannot know it is empty. `FRAMES <= MAX_FRAMES == 0` keeps the loops off it.
const BITMAP_WORDS: usize = if MAX_FRAMES == 0 { 1 } else { (MAX_FRAMES + 63) / 64 };

/// Why the allocator manages less than it was offered. Two constants rather than a
/// free-form message, so a test can assert *which* limit bound the range — the two
/// mean different things to whoever reads them: one is a property of the address
/// space, the other of a static array in this file. Neither states a figure, because
/// after 2.732 the bitmap's is per-board; [`describe`] prints it.
pub const BOUND_IDENTITY: &str = "the identity map ends there (paging::NORMAL_MAP_END)";
pub const BOUND_BITMAP: &str =
    "the tree offered more DRAM than this board declares (board::ACTIVE.ram_size)";

/// What the allocator would manage of a range, and what it would have to leave out.
pub struct Plan {
    /// First managed frame: `start` rounded **up** to a frame boundary.
    pub base: usize,
    /// Frames actually placed under management.
    pub frames: usize,
    /// Frames the range contained before the bounds were applied.
    pub offered: usize,
    /// Which limit bound the range, or `None` when the whole of it is managed.
    pub bound: Option<&'static str>,
}

impl Plan {
    /// Frames the bounds left unmanaged. Zero exactly when `bound` is `None`.
    pub fn dropped(&self) -> usize {
        self.offered - self.frames
    }

    /// First address past the managed frames.
    pub fn end(&self) -> usize {
        self.base + self.frames * FRAME_SIZE
    }
}

/// What can be managed of `[start, end)`, as a **pure function** of the range and the
/// address-space ceiling — the same shape as `board::resolve_from`, and for the same
/// reason: this machine boots with one range, so the bounds would otherwise be branches
/// nobody ever executes. `frame test` puts ranges in front of it that QEMU never gives
/// us.
///
/// Total by construction: an empty or inverted range yields zero frames rather than a
/// wrapped count, and rounding `start` up cannot overflow.
pub fn plan(start: usize, end: usize, map_end: usize) -> Plan {
    let base = start.saturating_add(FRAME_SIZE - 1) & !(FRAME_SIZE - 1);
    let frames_to = |hi: usize| if hi > base { (hi - base) / FRAME_SIZE } else { 0 };

    let offered = frames_to(end);
    // Bound 1 — the identity map. Memory the MMU does not translate is not memory we
    // may hand out, however truthfully the tree reported it.
    let mut frames = frames_to(end.min(map_end));
    let mut bound = if frames < offered { Some(BOUND_IDENTITY) } else { None };
    // Bound 2 — the bitmap. Applied second and allowed to overwrite the reason,
    // because what the operator needs named is the limit that actually bound the
    // range: the tighter of the two.
    if frames > MAX_FRAMES {
        frames = MAX_FRAMES;
        bound = Some(BOUND_BITMAP);
    }

    Plan { base, frames, offered, bound }
}

static mut BITMAP: [u64; BITMAP_WORDS] = [0; BITMAP_WORDS];
static mut BASE: usize = 0;
static mut FRAMES: usize = 0;
static mut OFFERED: usize = 0;
static mut BOUND: Option<&'static str> = None;

/// Initialize the allocator over the physical range `[start, end)`.
/// `start` is rounded up to a frame boundary.
///
/// The caller passes the range it wants managed; the bounds are applied here, where
/// they are enforced, and recorded so [`describe`] can name them.
pub fn init(start: usize, end: usize) {
    let p = plan(start, end, crate::paging::NORMAL_MAP_END);
    // SAFETY: single-threaded init, called once before IRQs/scheduling start.
    unsafe {
        BASE = p.base;
        FRAMES = p.frames;
        OFFERED = p.offered;
        BOUND = p.bound;
        let bm = &mut *addr_of_mut!(BITMAP);
        for w in bm.iter_mut() {
            *w = 0;
        }
    }
}

/// Allocate one physical frame; returns its physical address.
pub fn alloc() -> Option<usize> {
    // SAFETY: guarded by the single-owner invariant (frame alloc happens during
    // setup / with IRQs masked). No concurrent mutation on this single core.
    unsafe {
        let frames = FRAMES;
        let bm = &mut *addr_of_mut!(BITMAP);
        for i in 0..frames {
            let w = i / 64;
            let b = i % 64;
            if bm[w] & (1 << b) == 0 {
                bm[w] |= 1 << b;
                return Some(BASE + i * FRAME_SIZE);
            }
        }
        None
    }
}

/// Return a previously-allocated frame to the pool.
pub fn free(pa: usize) {
    // SAFETY: see `alloc`.
    unsafe {
        if pa < BASE {
            return;
        }
        let i = (pa - BASE) / FRAME_SIZE;
        if i >= FRAMES {
            return;
        }
        let bm = &mut *addr_of_mut!(BITMAP);
        bm[i / 64] &= !(1 << (i % 64));
    }
}

/// Total number of frames under management.
pub fn total_frames() -> usize {
    // SAFETY: read-only scalar.
    unsafe { FRAMES }
}

/// Frames currently handed out — counted from the bitmap itself, not from a parallel
/// tally that could drift from it (2.740).
///
/// Published because the heap can now take frames of its own ([`crate::heap::grow`]), and
/// "the heap grew by N frames" is only checkable against the allocator those frames came
/// from: without this, a frame counted as heap arena AND as free memory would read as the
/// machine having more RAM than it has.
pub fn used_frames() -> usize {
    // SAFETY: read-only traversal of the bitmap, same single-owner invariant as `alloc`.
    unsafe {
        let frames = FRAMES;
        let bm = &*addr_of_mut!(BITMAP);
        let mut n = 0usize;
        for i in 0..frames {
            if bm[i / 64] & (1 << (i % 64)) != 0 {
                n += 1;
            }
        }
        n
    }
}

/// First managed physical address.
pub fn base() -> usize {
    // SAFETY: read-only scalar.
    unsafe { BASE }
}

/// First address past the managed frames.
pub fn end() -> usize {
    // SAFETY: read-only scalars.
    unsafe { BASE + FRAMES * FRAME_SIZE }
}

/// Which limit bound the resolved range, or `None` when all of it is managed.
pub fn bound() -> Option<&'static str> {
    // SAFETY: read-only scalar.
    unsafe { BOUND }
}

/// Frames the bounds left unmanaged — the count that makes a bound a number rather
/// than a remark.
pub fn unmanaged_frames() -> usize {
    // SAFETY: read-only scalars.
    unsafe { OFFERED - FRAMES }
}

/// The allocator's range and where it came from (`frame`).
///
/// Deliberately says which of the DRAM figures is the *resolved* one, because after
/// 2.731 the honest answer to "how much memory does this machine have" and "how much
/// can this kernel hand out" are two different numbers, and the second one is the one
/// printed everywhere else.
pub fn describe() {
    let r = crate::board::resolved();
    let mib = |bytes: usize| bytes / (1024 * 1024);
    crate::kprintln!("frame allocator: bitmap over resolved DRAM");
    crate::kprintln!(
        "  resolved DRAM  {:#010x}..{:#010x} ({} MiB, {})",
        r.ram_base,
        r.ram_end(),
        r.ram_size / (1024 * 1024),
        r.ram_src.as_str()
    );
    crate::kprintln!(
        "  managed        {:#010x}..{:#010x} ({} MiB, {} frames)",
        base(),
        end(),
        mib(end() - base()),
        total_frames()
    );
    // The kernel image is not a bound — it is simply where the free memory starts —
    // so it is reported as the gap it is rather than counted as memory lost.
    crate::kprintln!(
        "  below the base  the kernel image, its stacks and page tables (never handed out)"
    );
    // The two ceilings, stated whether or not they bound this boot. A limit is only
    // checkable if the operator can see it before it fires (2.732) — and the bitmap's
    // is per-board now, so it cannot be read off the message when it does.
    crate::kprintln!(
        "  bitmap holds   {} frames ({} MiB) — this board's declared DRAM, clamped to the map",
        MAX_FRAMES,
        BITMAP_CAPACITY / (1024 * 1024)
    );
    crate::kprintln!(
        "  identity map   ends at {:#010x} — frames past it are translation faults",
        crate::paging::NORMAL_MAP_END
    );
    match bound() {
        Some(why) => {
            crate::kprintln!(
                "  ! BOUNDED: {} frames ({} MiB) of the resolved range are NOT managed",
                unmanaged_frames(),
                unmanaged_frames() * FRAME_SIZE / (1024 * 1024)
            );
            crate::kprintln!("    reason: {}", why);
            crate::kprintln!(
                "    (a bound is fine; a silent one is not — it would read as memory the"
            );
            crate::kprintln!("     machine does not have)");
        }
        None => crate::kprintln!("  every frame of the resolved range is under management"),
    }
    if let Some(why) = r.ram_refused {
        crate::kprintln!("  ! the tree's /memory claim was refused (rule 7): {}", why);
        crate::kprintln!("    the range above is the compile-time map's, not the tree's");
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

/// Exercise the bounds against synthetic ranges (`frame test`).
///
/// This machine boots with exactly one range, and it is a range both bounds happen to
/// accept — so without this, "the allocator refuses to hand out unmapped memory" would
/// be a claim about code that has never run. [`plan`] is pure, so the cases execute on
/// every machine and every build without disturbing the live allocator.
pub fn selftest() {
    const MIB: usize = 1024 * 1024;
    let ram = 0x4000_0000usize; // a DRAM base to hang the cases off
    // Every case below is written in multiples of the bitmap's capacity rather than in
    // megabytes (2.732). The capacity is per-board now, so "512 MiB overflows the
    // bitmap" was a fact about `board-virt` that would have quietly stopped being a
    // fact — and stopped testing anything — on a build whose bitmap is larger.
    let cap = BITMAP_CAPACITY;
    let map_end = ram + 4 * cap; // a Normal window comfortably wider than the bitmap
    let (mut pass, mut total) = (0u32, 0u32);
    crate::kprintln!("frame allocator: bound self-test against synthetic ranges");

    let p = plan(ram, ram + cap / 4, map_end);
    case_ok(&mut pass, &mut total,
        p.bound.is_none() && p.frames == cap / 4 / FRAME_SIZE && p.dropped() == 0
            && p.base == ram && p.end() == ram + cap / 4,
        "a range inside both bounds is managed whole, and says so");

    // The kernel image ends wherever the linker left it; the first frame must start
    // above it, never on top of it.
    let p = plan(ram + 1, ram + cap / 4, map_end);
    case_ok(&mut pass, &mut total,
        p.base == ram + FRAME_SIZE && p.base % FRAME_SIZE == 0,
        "an unaligned start rounds UP, so no frame overlaps the kernel image");

    let p = plan(ram + cap / 4, ram, map_end);
    case_ok(&mut pass, &mut total, p.frames == 0 && p.offered == 0 && p.bound.is_none(),
        "an inverted range yields zero frames, not a wrapped count");

    // Bound 1: the identity map. The range must STRADDLE the ceiling with less than
    // the bitmap's capacity of window above the base — offering four capacities behind
    // a one-capacity window would be bounded by the *bitmap* instead, that being the
    // tighter limit. So bound 1 binds first exactly when DRAM runs past the top of the
    // Normal window with under a bitmap's worth of window left: half a capacity of
    // window here, two capacities of DRAM offered.
    let straddle = map_end - cap / 2;
    let p = plan(straddle, straddle + 2 * cap, map_end);
    case_ok(&mut pass, &mut total,
        p.bound == Some(BOUND_IDENTITY) && p.end() == map_end && p.frames < MAX_FRAMES,
        "DRAM past the identity map is bounded, and the bound is NAMED");

    let p = plan(straddle, straddle + 2 * cap, map_end);
    case_ok(&mut pass, &mut total,
        p.dropped() == p.offered - p.frames && p.dropped() == (3 * cap / 2) / FRAME_SIZE,
        "a bound reports the frames it cost, not just that it applied");

    // Bound 2: the bitmap. Half a capacity more DRAM than this file can cover, inside a
    // window wide enough that the map is not what bound it.
    let over = ram + cap + cap / 2;
    let p = plan(ram, over, map_end);
    case_ok(&mut pass, &mut total,
        p.bound == Some(BOUND_BITMAP) && p.frames == MAX_FRAMES
            && p.dropped() == (cap / 2) / FRAME_SIZE,
        "DRAM past the static bitmap is bounded, and the bound is a DIFFERENT reason");

    // Exactly the bitmap's capacity: the off-by-one that would cry wolf on a machine
    // whose memory fits perfectly.
    let p = plan(ram, ram + cap, map_end);
    case_ok(&mut pass, &mut total, p.bound.is_none() && p.frames == MAX_FRAMES,
        "a range of exactly the bitmap's capacity is NOT reported as bounded");

    // Both bind at once. The operator is owed the one that actually held.
    let p = plan(ram, ram + 16 * cap, map_end);
    case_ok(&mut pass, &mut total,
        p.bound == Some(BOUND_BITMAP) && p.frames == MAX_FRAMES,
        "when both bounds bind, the TIGHTER one is the reason reported");

    // The whole point of bound 1: never hand out an address the MMU cannot translate.
    let p = plan(map_end, map_end + cap, map_end);
    case_ok(&mut pass, &mut total, p.frames == 0 && p.bound == Some(BOUND_IDENTITY),
        "a range entirely past the identity map manages nothing (and says why)");

    let p = plan(ram, ram + 16 * cap, map_end);
    case_ok(&mut pass, &mut total,
        p.end() <= map_end && p.frames <= MAX_FRAMES,
        "no plan ever exceeds the identity map or the bitmap, whatever it is offered");

    // 2.732's own two claims, asserted against the REAL board map and the REAL ceiling
    // rather than the synthetic ones above — these are the cases that would have failed
    // before the bitmap was sized from `board::ACTIVE`.
    let m = crate::board::ACTIVE;
    let real_end = crate::paging::NORMAL_MAP_END;
    let p = plan(m.ram_base as usize, m.ram_base as usize + m.ram_size as usize, real_end);
    case_ok(&mut pass, &mut total, p.bound != Some(BOUND_BITMAP),
        "this board's OWN declared DRAM is never bitmap-bounded (a 256 MiB constant bound rpi4/pine64)");

    case_ok(&mut pass, &mut total,
        BITMAP_CAPACITY <= real_end.saturating_sub(m.ram_base as usize)
            && (BITMAP_CAPACITY <= m.ram_size as usize),
        "the bitmap reserves bits for no frame the map cannot translate, nor past what the board declares");

    crate::kprintln!(
        "frame allocator: {}/{} bounds held (bitmap {} frames = {} MiB, from board::ACTIVE)",
        pass, total, MAX_FRAMES, BITMAP_CAPACITY / MIB
    );
}
