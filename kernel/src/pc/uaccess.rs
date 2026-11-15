//! The PC kernel's one door for user memory (gap G2 part 3, docs/security-model.md). Every system call that reads
//! or writes a ring-3 buffer goes through `from_user` or `to_user`; no other syscall code follows a user address.
//! A new pointer-taking call adds its case to the `uptr` program (pc/proc.rs, suite pc-userptr) in the same commit.
//!
//! Two guards, each enough alone for its own job:
//! 1. **The page walk.** The range must lie in the canonical lower half, must not wrap, and every page of it must be
//!    present and USER at all four levels of the CALLER's table (the current CR3: a process's own space is current
//!    during its system call). The kernel's own pages never carry the USER bit, so a kernel address is refused here
//!    even if a caller's own range check is wrong or missing. The walk runs before any byte moves, so a copy is
//!    all or nothing: a refusal leaves the destination untouched and never returns a short length.
//! 2. **SMAP** (CR4 bit 21), where CPUID leaf 7 EBX bit 20 says the CPU has it. The kernel then faults on any user
//!    page it touches outside the `stac`..`clac` window in `copy`, so a future code path that forgets this door
//!    stops instead of silently reading a program's memory. The fault handler names it (`smap_violation`).

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

/// AFR-0001 7.2: a buffer outside the caller's memory.
pub const E_FAULT: i64 = -4;

/// The lower canonical half ends here; ring-3 memory lives below it.
const LOWER_HALF_END: u64 = 0x0000_8000_0000_0000;

static SMAP_ON: AtomicBool = AtomicBool::new(false);

/// Turn SMAP on if the CPU has it, and say which. Called once, after the kernel's own page tables are loaded (the
/// firmware's tables may mark kernel pages USER; the kernel's never do).
pub fn init() {
    // SAFETY: CPUID leaf 7 subleaf 0 exists on every x86-64 CPU this kernel boots on (leaf 0's max is checked).
    let has = unsafe {
        let max = core::arch::x86_64::__cpuid(0).eax;
        max >= 7 && core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 20) != 0
    };
    if has {
        // SAFETY: CR4.SMAP on a CPU that reports it; the kernel's tables give no kernel page the USER bit, and every
        // syscall copy of user memory runs inside `copy`'s stac..clac window.
        unsafe {
            let mut cr4: u64;
            asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
            cr4 |= 1 << 21;
            asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack, preserves_flags));
        }
        SMAP_ON.store(true, Ordering::Relaxed);
        crate::kprintln!("[pc] SMAP: on (CR4.SMAP): the kernel touches ring-3 memory only through pc/uaccess.rs");
    } else {
        crate::kprintln!("[pc] SMAP: this CPU does not offer it (CPUID 7 EBX bit 20); ring-3 copies rely on the page walk");
    }
}

pub fn smap_on() -> bool {
    SMAP_ON.load(Ordering::Relaxed)
}

/// Guard 1: may the kernel move `len` bytes at `ptr` for the program whose space is current? A write also needs
/// every page writable for ring 3 (the code pages are read-only: a kernel write there would fault and halt).
pub fn user_pages_ok(ptr: u64, len: u64, write: bool) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = ptr.checked_add(len) else { return false };
    if end > LOWER_HALF_END {
        return false;
    }
    let root = crate::mmu::cr3() & 0x000F_FFFF_FFFF_F000;
    let mut pg = ptr & !0xFFF;
    while pg < end {
        let ok = if write { crate::mmu::user_writable_in(root, pg) } else { crate::mmu::user_mapped_in(root, pg) };
        if !ok {
            return false;
        }
        pg += 4096;
    }
    true
}

/// Copy `n` bytes inside the SMAP window. Both ranges have been checked by the caller (`from_user` / `to_user`).
unsafe fn copy(src: *const u8, dst: *mut u8, n: usize) {
    let smap = smap_on();
    if smap {
        // SAFETY: stac only sets RFLAGS.AC; the CPU has SMAP (checked at init).
        unsafe { asm!("stac", options(nomem, nostack)) };
    }
    // SAFETY: the caller checked both ranges.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, n) };
    if smap {
        // SAFETY: clac only clears RFLAGS.AC.
        unsafe { asm!("clac", options(nomem, nostack)) };
    }
}

/// Read `dst.len()` bytes from the caller's memory at `ptr`: all of them, or `Err(E_FAULT)` with `dst` untouched.
pub fn from_user(ptr: u64, dst: &mut [u8]) -> Result<(), i64> {
    if !user_pages_ok(ptr, dst.len() as u64, false) {
        return Err(E_FAULT);
    }
    // SAFETY: every page of [ptr, ptr+len) is present and USER in the current space (checked), and `dst` is ours.
    unsafe { copy(ptr as *const u8, dst.as_mut_ptr(), dst.len()) };
    Ok(())
}

/// Write `src` into the caller's memory at `ptr`: all of it, or `Err(E_FAULT)` with nothing written.
pub fn to_user(ptr: u64, src: &[u8]) -> Result<(), i64> {
    if !user_pages_ok(ptr, src.len() as u64, true) {
        return Err(E_FAULT);
    }
    // SAFETY: every page of [ptr, ptr+len) is present, USER and writable in the current space (checked).
    unsafe { copy(src.as_ptr(), ptr as *mut u8, src.len()) };
    Ok(())
}

/// The #PF handler's question: was this a kernel touch of a ring-3 page that SMAP stopped? (Kernel mode, the page
/// present, the address in the lower half, and SMAP on.)
pub fn smap_violation(cs: u64, error: u64, cr2: u64) -> bool {
    cs & 3 == 0 && error & 1 != 0 && cr2 < LOWER_HALF_END && cr2 >= crate::mmu::USER_BASE && smap_on()
}
