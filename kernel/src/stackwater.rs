//! F7, step 1 (measure first): how deep do the two boot stacks really go?
//!
//! The boot stack (`__stack_top`, 1 MiB since 6e00ee98) and the secondary core's stack
//! (`__stack_top_secondary`, 64 KiB) have no guard: an overrun runs silently into whatever lies below
//! (the top of `.bss` for the boot stack, the top of the boot stack for the secondary). 6e00ee98 found
//! that 512 KiB was too small for the debug shell. This module paints each stack with a known word at
//! boot and later reports the lowest address that is no longer painted: the high-water mark. It changes
//! no mapping and catches nothing; it is the measurement a guard page has to be sized from.
//!
//! Limits, stated plainly: a frame that is reserved but never written (a big local array the code
//! never touches) does not show; a callee that happens to write the paint word back is undercounted by
//! that word. Both make the number a LOWER bound on use.

#![allow(dead_code)] // the Pi build compiles the module but never calls it

use core::arch::asm;
use core::ptr::addr_of;

/// The paint word. Not zero (zeroed memory must not look untouched) and not a likely pointer.
const PAINT: u64 = 0x5afe_57ac_5afe_57ac;

/// Boot-stack bytes left unpainted under the live SP when painting (the frames of this very call).
const LIVE_MARGIN: usize = 1024;

extern "C" {
    static __stack_bottom: u8;
    static __stack_top: u8;
    static __sec_guard_hi: u8;
    static __stack_top_secondary: u8;
}

/// Size of the secondary core's stack (the `. += 0x10000` in kernel/linker.ld).
pub const SECONDARY_SIZE: usize = 0x10000;

/// One stack's extent: [bottom, top).
#[derive(Clone, Copy)]
pub struct Extent {
    pub bottom: usize,
    pub top: usize,
}

/// The boot stack: `__stack_bottom` (just above its guard) up to `__stack_top` (kernel/linker.ld, F7 layout).
pub fn boot() -> Extent {
    Extent { bottom: addr_of!(__stack_bottom) as usize, top: addr_of!(__stack_top) as usize }
}

/// The secondary core's stack: from just above its guard up to `__stack_top_secondary`.
pub fn secondary() -> Extent {
    Extent { bottom: addr_of!(__sec_guard_hi) as usize, top: addr_of!(__stack_top_secondary) as usize }
}

fn sp() -> usize {
    let v: usize;
    // SAFETY: reads the stack pointer only.
    unsafe { asm!("mov {}, sp", out(reg) v, options(nomem, nostack)) };
    v
}

fn paint(from: usize, to: usize) {
    let mut a = (from + 7) & !7;
    while a + 8 <= to {
        // SAFETY: [from, to) is stack memory no live frame uses (callers keep a margin under SP).
        unsafe { core::ptr::write_volatile(a as *mut u64, PAINT) };
        a += 8;
    }
}

/// Clean and invalidate [from, to) to the point of coherency, so a core that starts with its MMU and
/// caches off (the secondary, at `_start_secondary`) sees the paint, and its uncached writes are not
/// later hidden behind the primary's stale cache lines.
fn clean_invalidate(from: usize, to: usize) {
    let mut a = from & !63;
    while a < to {
        // SAFETY: data-cache maintenance by VA on mapped Normal memory.
        unsafe { asm!("dc civac, {}", in(reg) a, options(nostack)) };
        a += 64;
    }
    // SAFETY: barrier only.
    unsafe { asm!("dsb sy", options(nostack)) };
}

/// Paint the unused part of the boot stack. Call once, early in `kernel_main` (after the MMU is on).
pub fn paint_boot() {
    let e = boot();
    let below_live = sp().saturating_sub(LIVE_MARGIN);
    if below_live > e.bottom && below_live <= e.top {
        paint(e.bottom, below_live);
    }
}

/// Paint the whole secondary stack. Call on the primary BEFORE PSCI CPU_ON starts core 1.
pub fn paint_secondary() {
    let e = secondary();
    paint(e.bottom, e.top);
    clean_invalidate(e.bottom, e.top);
}

/// Lowest address in `e` whose word is not the paint, scanning up from the bottom. `e.top` = never used.
fn low_water(e: Extent) -> usize {
    let mut a = (e.bottom + 7) & !7;
    while a + 8 <= e.top {
        // SAFETY: reading stack memory as plain words; never written through this pointer.
        if unsafe { core::ptr::read_volatile(a as *const u64) } != PAINT {
            return a;
        }
        a += 8;
    }
    e.top
}

/// Bytes of `e` used at the deepest point so far.
pub fn used(e: Extent) -> usize {
    e.top - low_water(e)
}

/// `stack`: both stacks, size, deepest use so far and what is left, in one line each.
pub fn report() {
    let rows: [(&str, Extent); 2] = [("boot", boot()), ("secondary", secondary())];
    for (name, e) in rows {
        let size = e.top - e.bottom;
        let u = used(e);
        crate::kprintln!(
            "[stack] {} {:#x}..{:#x} size={} KiB used={} B ({}%) free={} B{}",
            name,
            e.bottom,
            e.top,
            size / 1024,
            u,
            u * 100 / size.max(1),
            size - u,
            if u >= size { "  OVERRUN: no paint left at the bottom" } else { "" }
        );
    }
    crate::kprintln!("[stack] sp now {:#x} (boot top {:#x})", sp(), boot().top);
}

/// `stack guards`: ask the MMU (AT S1E1R) about the first and last byte of each guard and the first byte
/// of each stack. A guard must FAULT, a stack must be MAPPED.
pub fn check_guards() -> bool {
    let mut ok = true;
    for (name, lo, hi) in crate::paging::guards() {
        let g_lo = crate::paging::translates(lo);
        let g_hi = crate::paging::translates(hi - 1);
        let st = crate::paging::translates(hi);
        let good = !g_lo && !g_hi && st;
        ok &= good;
        crate::kprintln!(
            "[stack] guard {} {:#x}..{:#x}: first byte {}, last byte {}, stack above it {} -> {}",
            name,
            lo,
            hi,
            if g_lo { "MAPPED" } else { "faults" },
            if g_hi { "MAPPED" } else { "faults" },
            if st { "mapped" } else { "FAULTS" },
            if good { "OK" } else { "NOT GUARDED" }
        );
    }
    crate::kprintln!("[stack] guards {}", if ok { "OK" } else { "NOT OK" });
    ok
}

/// One level of the deliberate overrun: a 4 KiB local that is really written, then recurse.
#[inline(never)]
fn dive(depth: usize) -> usize {
    let mut pad = [0u8; 4096];
    for (i, b) in pad.iter_mut().enumerate().step_by(512) {
        // SAFETY: a valid &mut u8 in this frame; volatile so the frame cannot be optimised away.
        unsafe { core::ptr::write_volatile(b, (depth + i) as u8) };
    }
    if depth % 32 == 0 {
        crate::kprintln!("[stack] overflow-test depth {} sp {:#x}", depth, sp());
    }
    // SAFETY: reading back one byte of this frame.
    let x = unsafe { core::ptr::read_volatile(&pad[0]) } as usize;
    x + dive(depth + 1)
}

/// `stack overflow-test`: recurse on the boot stack until it runs out. With the F7 guard this must end in
/// `[mansa] STACK OVERFLOW: boot stack ...` and a halt; without it, the dive runs on into `.bss`.
pub fn overflow_test() {
    crate::kprintln!("[stack] overflow-test: diving from sp {:#x}, stack bottom {:#x}", sp(), boot().bottom);
    let r = dive(0);
    crate::kprintln!("[stack] overflow-test returned {} (should be unreachable)", r);
}
