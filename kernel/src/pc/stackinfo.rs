//! F8, step 1 (measure first): where the PC kernel's stack is and how deep it goes.
//!
//! The PC kernel never sets up a stack of its own: `efi_main` runs on the stack the UEFI firmware gave the
//! application, in boot-services memory, and stays on it after ExitBootServices. Nothing below it is guarded.
//! The ARM kernel's debug-build `shell::exec` frame is 595 KiB (docs/evidence-f7-stackwater.log) and the PC
//! shares shell.rs, so the PC stack may well be overrun today without anyone seeing it.
//!
//! This module records RSP as `efi_main` starts, finds the memory-map region that holds it, and once the kernel
//! runs on its own page tables paints the region below the live stack (at most `PAINT_MAX`, never below 1 MiB).
//! `stack` then reports the deepest point reached below `efi_main`'s entry. It changes no mapping and catches
//! nothing. The figure is a LOWER bound (a reserved but never-written frame does not show), and use above the
//! paint point (the frames between `efi_main` and the paint call) is counted as the distance from entry RSP.

use core::sync::atomic::{AtomicU64, Ordering};

const PAINT: u64 = 0x5afe_57ac_5afe_57ac;
/// Paint at most this far below the stack pointer (the region holding the stack may be far larger).
const PAINT_MAX: u64 = 4 << 20;
/// Leave this much under the live SP unpainted (the frames of the paint call itself).
const LIVE_MARGIN: u64 = 8192;
const ONE_MIB: u64 = 1 << 20;

static ENTRY_RSP: AtomicU64 = AtomicU64::new(0);
static REGION_START: AtomicU64 = AtomicU64::new(0);
static REGION_END: AtomicU64 = AtomicU64::new(0);
static REGION_TYPE: AtomicU64 = AtomicU64::new(u64::MAX);
static PAINT_LO: AtomicU64 = AtomicU64::new(0);
static PAINT_HI: AtomicU64 = AtomicU64::new(0);

fn rsp() -> u64 {
    let v: u64;
    // SAFETY: reads the stack pointer only.
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) v, options(nomem, nostack)) };
    v
}

/// First thing in `efi_main`: remember where the firmware's stack pointer was.
pub fn note_entry() {
    ENTRY_RSP.store(rsp(), Ordering::Relaxed);
}

/// For each memory-map descriptor, before ExitBootServices: keep the one that holds the entry RSP.
pub fn note_region(start: u64, pages: u64, ty: u32) {
    let r = ENTRY_RSP.load(Ordering::Relaxed);
    let end = start + pages * 4096;
    if r >= start && r < end {
        REGION_START.store(start, Ordering::Relaxed);
        REGION_END.store(end, Ordering::Relaxed);
        REGION_TYPE.store(ty as u64, Ordering::Relaxed);
    }
}

/// After the kernel's own page tables are live (the firmware's may sit in this very region until then):
/// paint below the live stack, inside the region, never below 1 MiB, at most PAINT_MAX.
pub fn paint() {
    let start = REGION_START.load(Ordering::Relaxed);
    let ty = REGION_TYPE.load(Ordering::Relaxed);
    // Only boot-services code/data (types 3, 4) or conventional memory (7): free once the firmware has gone.
    if !matches!(ty, 3 | 4 | 7) {
        return;
    }
    let hi = (rsp().saturating_sub(LIVE_MARGIN)) & !7;
    let lo = start.max(ONE_MIB).max(hi.saturating_sub(PAINT_MAX)) & !7;
    if lo >= hi {
        return;
    }
    let mut a = lo;
    while a < hi {
        // SAFETY: [lo, hi) is below the live stack, inside the firmware region that held it, which nothing
        // else uses after ExitBootServices (the frame pool and heap live inside the image), and above 1 MiB
        // (the second core's start-up page is below 640 KiB). Mapped 1:1 by mmu::init.
        unsafe { core::ptr::write_volatile(a as *mut u64, PAINT) };
        a += 8;
    }
    PAINT_LO.store(lo, Ordering::Relaxed);
    PAINT_HI.store(hi, Ordering::Relaxed);
}

/// The lowest address in the painted range that is no longer paint (`hi` if none was touched).
fn low_water() -> u64 {
    let (lo, hi) = (PAINT_LO.load(Ordering::Relaxed), PAINT_HI.load(Ordering::Relaxed));
    let mut a = lo;
    while a < hi {
        // SAFETY: inside the painted range, read as plain words.
        if unsafe { core::ptr::read_volatile(a as *const u64) } != PAINT {
            return a;
        }
        a += 8;
    }
    hi
}

// ---------------------------------------------------------------------------------------------------------------
// F8 step 2 (docs/design-f8-pc-kernel-stack.md): the kernel's own stack, with an unmapped guard below it.

/// Guard below the kernel stack (unmapped by mmu::guard).
pub const GUARD: usize = 64 << 10;
/// The kernel stack. shell::exec's debug frame is 590 KiB (llvm-objdump, `__chkstk` of 0x93a48), and `report`
/// calls exec again from inside exec, so two frames nest: 1.18 MiB, past the first 1 MiB (pc-report hit the
/// guard on the public walk, Actions 42958601929). 2 MiB holds that with room; shrinking exec is the real fix.
pub const STACK: usize = 2 << 20;

#[repr(C, align(4096))]
struct Area([u8; GUARD + STACK]);
static mut AREA: Area = Area([0; GUARD + STACK]);

#[repr(C, align(16))]
pub struct DfStack(pub [u8; 16 << 10]);
/// The #DF handler's own stack (TSS IST 1): a fault with RSP inside the guard cannot push anywhere else.
pub static mut DF_STACK: DfStack = DfStack([0; 16 << 10]);

/// [lo, hi) of the guard.
pub fn guard() -> (u64, u64) {
    let lo = core::ptr::addr_of!(AREA) as u64;
    (lo, lo + GUARD as u64)
}

/// [bottom, top) of the kernel stack.
pub fn kstack() -> (u64, u64) {
    let (_, bottom) = guard();
    (bottom, bottom + STACK as u64)
}

static ON_KSTACK: AtomicU64 = AtomicU64::new(0);

core::arch::global_asm!(
    ".global pc_switch_stack",
    "pc_switch_stack:", // sysv64: rdi = new stack top (16-byte aligned), rsi = fn() -> !
    "    mov rsp, rdi",
    "    xor ebp, ebp",
    "    call rsi",
    "    ud2",
);
extern "sysv64" {
    fn pc_switch_stack(top: u64, f: extern "sysv64" fn() -> !) -> !;
}

/// Paint the kernel stack, unmap its guard, and continue in `f` on it. Never returns.
pub fn run_on_kernel_stack(f: extern "sysv64" fn() -> !) -> ! {
    let (bottom, top) = kstack();
    let mut a = bottom;
    while a < top {
        // SAFETY: the kernel stack is a static of this module that nothing runs on yet.
        unsafe { core::ptr::write_volatile(a as *mut u64, PAINT) };
        a += 8;
    }
    let (glo, ghi) = guard();
    let guarded = crate::mmu::guard(glo, ghi);
    crate::kprintln!(
        "[pc] kernel stack: {} KiB at {:#x}..{:#x}, guard {:#x}..{:#x} {}",
        STACK >> 10,
        bottom,
        top,
        glo,
        ghi,
        if guarded { "unmapped" } else { "NOT unmapped (out of table frames)" }
    );
    ON_KSTACK.store(1, Ordering::Relaxed);
    // SAFETY: `top` is the 16-byte-aligned end of a static stack nothing else uses; `f` never returns.
    unsafe { pc_switch_stack(top & !15, f) }
}

fn kstack_low_water() -> u64 {
    let (bottom, top) = kstack();
    let mut a = bottom;
    while a < top {
        // SAFETY: inside the kernel stack, read as plain words.
        if unsafe { core::ptr::read_volatile(a as *const u64) } != PAINT {
            return a;
        }
        a += 8;
    }
    top
}

/// `stack guards` on the PC: the MMU's answer for every guard page and the stack's first page.
pub fn check_guards() -> bool {
    let (lo, hi) = guard();
    let mut mapped = 0;
    let mut p = lo;
    while p < hi {
        if crate::mmu::translate(p).is_some() {
            mapped += 1;
        }
        p += 4096;
    }
    let stack_ok = crate::mmu::translate(hi).is_some();
    let ok = mapped == 0 && stack_ok;
    crate::kprintln!(
        "[stack] pc guard {:#x}..{:#x}: {} of {} pages mapped, stack above it {} -> {}",
        lo,
        hi,
        mapped,
        (hi - lo) / 4096,
        if stack_ok { "mapped" } else { "UNMAPPED" },
        if ok { "OK" } else { "NOT GUARDED" }
    );
    ok
}

#[inline(never)]
fn dive(depth: usize) -> usize {
    let mut pad = [0u8; 4096];
    for (i, b) in pad.iter_mut().enumerate().step_by(512) {
        // SAFETY: a valid &mut u8 in this frame; volatile so the frame stays.
        unsafe { core::ptr::write_volatile(b, (depth + i) as u8) };
    }
    if depth % 32 == 0 {
        crate::kprintln!("[stack] overflow-test depth {} rsp {:#x}", depth, rsp());
    }
    // SAFETY: reading back one byte of this frame.
    let x = unsafe { core::ptr::read_volatile(&pad[0]) } as usize;
    x + dive(depth + 1)
}

/// `stack overflow-test` on the PC: run the kernel stack out on purpose; with the guard it must end in
/// `[pc] STACK OVERFLOW` and a halt.
pub fn overflow_test() {
    crate::kprintln!("[stack] overflow-test: diving from rsp {:#x}, stack bottom {:#x}", rsp(), kstack().0);
    let r = dive(0);
    crate::kprintln!("[stack] overflow-test returned {} (should be unreachable)", r);
}

/// The guard check for the trap path: is `addr` inside the kernel stack's guard?
pub fn in_guard(addr: u64) -> bool {
    let (lo, hi) = guard();
    addr >= lo && addr < hi
}

/// `stack` on the PC: the firmware stack the boot ran on, and the kernel stack the shell runs on.
pub fn report() {
    let entry = ENTRY_RSP.load(Ordering::Relaxed);
    let (start, end, ty) =
        (REGION_START.load(Ordering::Relaxed), REGION_END.load(Ordering::Relaxed), REGION_TYPE.load(Ordering::Relaxed));
    let (lo, hi) = (PAINT_LO.load(Ordering::Relaxed), PAINT_HI.load(Ordering::Relaxed));
    crate::kprintln!(
        "[stack] pc firmware stack: entry rsp {:#x} in memory-map region {:#x}..{:#x} (type {}, {} KiB, {} KiB below entry rsp)",
        entry,
        start,
        end,
        ty,
        (end - start) / 1024,
        entry.saturating_sub(start) / 1024
    );
    if lo < hi {
        let low = low_water();
        crate::kprintln!(
            "[stack] pc boot path used={} B below entry rsp on the firmware stack (painted {:#x}..{:#x}){}",
            entry.saturating_sub(low),
            lo,
            hi,
            if low == lo { "  OVERRUN: no paint left at the bottom of the painted range" } else { "" }
        );
    }
    let now = rsp();
    let (bottom, top) = kstack();
    let on = now >= bottom && now < top;
    if ON_KSTACK.load(Ordering::Relaxed) != 0 {
        let used = top - kstack_low_water().min(now);
        crate::kprintln!(
            "[stack] pc kernel stack {:#x}..{:#x} size={} KiB used={} B ({}%) free={} B",
            bottom,
            top,
            STACK >> 10,
            used,
            used * 100 / STACK as u64,
            STACK as u64 - used
        );
    }
    crate::kprintln!(
        "[stack] pc rsp now {:#x}: {}",
        now,
        if on {
            "inside the kernel stack"
        } else if now >= start && now < end {
            "inside the firmware stack region"
        } else {
            "OUTSIDE every stack (below the firmware's region: memory that belongs to something else)"
        }
    );
}
