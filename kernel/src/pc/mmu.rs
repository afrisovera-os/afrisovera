//! The kernel's own page tables (x86-64, 4 levels: PML4 -> PDPT -> PD -> PT).
//!
//! Until `init` the CPU walks the firmware's tables, which sit in boot-services memory the
//! kernel will want back. `init` builds the kernel's own and loads them into CR3:
//! every physical address below `top` (at least 4 GiB, so the framebuffer, the local APIC
//! and the IOAPIC are covered) is mapped 1:1 with 2 MiB pages, so every pointer the kernel
//! already holds — its image, its stack, the screen — stays valid across the switch.
//! The local-APIC/IOAPIC block is mapped uncached. Table frames come from the frame pool.
//!
//! Above that, the DEMAND window holds nothing at boot: a touch there page-faults, and
//! `page_fault` maps a fresh zeroed 4 KiB frame and lets the instruction run again. That is
//! the proof the kernel now owns translation (a fault taken, served, retried), and the
//! mechanism later steps build on (user address spaces, lazy stacks).

use crate::frame;
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

const PRESENT: u64 = 1 << 0;
const WRITE: u64 = 1 << 1;
const USER: u64 = 1 << 2; // ring 3 may use it (needed at every level of the walk)
const PWT: u64 = 1 << 3;
const PCD: u64 = 1 << 4;
const HUGE: u64 = 1 << 7; // in a PD entry: a 2 MiB page
const ADDR: u64 = 0x000F_FFFF_FFFF_F000;
const PAGE_2M: u64 = 2 << 20;

/// The local APIC (0xFEE00000) and IOAPIC (0xFEC00000) live in this 2 MiB-aligned block.
const APIC_BLOCK: core::ops::Range<u64> = 0xFEC0_0000..0xFF00_0000;

/// Nothing is mapped here at boot; faults inside are served on demand. 512 GiB up: above
/// any RAM this kernel maps 1:1 (`init` refuses a `top` that reaches it).
pub const DEMAND_BASE: u64 = 0x0000_0080_0000_0000;
pub const DEMAND_SIZE: u64 = 64 << 20;

pub static DEMAND_FAULTS: AtomicU64 = AtomicU64::new(0);
static ROOT: AtomicU64 = AtomicU64::new(0);

/// The physical address of the kernel's PML4 (0 before `init`).
pub fn root() -> u64 {
    ROOT.load(Ordering::Relaxed)
}

pub fn cr3() -> u64 {
    let v: u64;
    // SAFETY: reading CR3 has no side effects.
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack)) };
    v
}

fn table(pa: u64) -> &'static mut [u64; 512] {
    // SAFETY: every table frame comes from the frame pool, which is inside the 1:1 map both
    // before and after the switch, and is owned by these tables alone.
    unsafe { &mut *(pa as *mut [u64; 512]) }
}

fn index(va: u64, level: u32) -> usize {
    ((va >> (12 + 9 * (level - 1))) & 511) as usize
}

/// The entry for `va` at `level` (2 = a PD entry / 2 MiB page, 1 = a PT entry / 4 KiB page),
/// creating any missing table on the way down. None when out of frames, or when a 2 MiB
/// page already covers `va` and so there is no lower table to reach.
fn entry(va: u64, level: u32, user: bool) -> Option<&'static mut u64> {
    entry_in(root(), va, level, user)
}

/// `entry`, in the address space whose PML4 is `root`.
fn entry_in(root: u64, va: u64, level: u32, user: bool) -> Option<&'static mut u64> {
    let mut t = root;
    let mut l = 4;
    let u = if user { USER } else { 0 };
    loop {
        let e = &mut table(t)[index(va, l)];
        if l == level {
            return Some(e);
        }
        if *e & PRESENT == 0 {
            *e = frame::alloc()? as u64 | PRESENT | WRITE | u; // alloc hands back a zeroed frame
        } else if user && *e & USER == 0 {
            return None; // a kernel-only table on the way: never open it to ring 3
        } else if *e & HUGE != 0 {
            return None;
        }
        t = *e & ADDR;
        l -= 1;
    }
}

/// Where `va` lands in physical memory under the kernel's tables, or None if unmapped.
pub fn translate(va: u64) -> Option<u64> {
    let mut t = root();
    if t == 0 {
        return None;
    }
    let mut l = 4;
    loop {
        let e = table(t)[index(va, l)];
        if e & PRESENT == 0 {
            return None;
        }
        if l == 1 || (l == 2 && e & HUGE != 0) {
            let size = if l == 1 { 1u64 << 12 } else { PAGE_2M };
            return Some((e & ADDR & !(size - 1)) | (va & (size - 1)));
        }
        t = e & ADDR;
        l -= 1;
    }
}

/// Build the 1:1 map of [0, top) and switch CR3 to it. Returns (bytes mapped, table frames).
pub fn init(top: u64) -> Option<(u64, usize)> {
    let top = (top.max(4 << 30) + PAGE_2M - 1) & !(PAGE_2M - 1);
    if top > DEMAND_BASE {
        return None;
    }
    let before = frame::used_frames();
    ROOT.store(frame::alloc()? as u64, Ordering::Relaxed);
    let mut pa = 0;
    while pa < top {
        let cache = if APIC_BLOCK.contains(&pa) { PCD | PWT } else { 0 };
        *entry(pa, 2, false)? = pa | PRESENT | WRITE | HUGE | cache;
        pa += PAGE_2M;
    }
    // SAFETY: the new tables map everything the kernel is using 1:1 (image, stack, pool,
    // framebuffer, APIC), exactly as the firmware's did, so execution continues unchanged.
    unsafe { asm!("mov cr3, {}", in(reg) root(), options(nostack)) };
    Some((top, frame::used_frames() - before))
}

/// Map device registers at [pa, pa + len) 1:1, uncached, in 2 MiB pages: for a PCI BAR the
/// firmware placed above the RAM map (on q35 the 64-bit window starts far above 4 GiB).
/// Pages already mapped are left as they are. False when out of table frames.
pub fn map_mmio(pa: u64, len: u64) -> bool {
    let mut p = pa & !(PAGE_2M - 1);
    while p < pa + len {
        if translate(p).is_none() {
            match entry(p, 2, false) {
                Some(e) => *e = p | PRESENT | WRITE | HUGE | PCD | PWT,
                None => return false,
            }
            // SAFETY: drops any stale translation for the page just mapped.
            unsafe { asm!("invlpg [{}]", in(reg) p, options(nostack)) };
        }
        p += PAGE_2M;
    }
    true
}

/// Called from the #PF handler. Serves a not-present fault inside the demand window with a
/// fresh frame and returns true (the faulting instruction then runs again); anything else
/// returns false and the handler reports the fault as fatal.
pub fn page_fault(addr: u64, error: u64) -> bool {
    // error bit 2: the fault came from ring 3, which the demand window is not for
    if root() == 0 || error & (PRESENT | USER) != 0 || !(DEMAND_BASE..DEMAND_BASE + DEMAND_SIZE).contains(&addr) {
        return false;
    }
    let Some(fr) = frame::alloc() else { return false };
    match entry(addr & !0xFFF, 1, false) {
        Some(e) => {
            // Not-present entries are never cached in the TLB, so no flush is needed.
            *e = fr as u64 | PRESENT | WRITE;
            DEMAND_FAULTS.fetch_add(1, Ordering::Relaxed);
            true
        }
        None => {
            frame::free(fr);
            false
        }
    }
}

/// Where ring-3 programs live: a PML4 slot of their own (1 TiB up), so no table on the way
/// is shared with the kernel's 1:1 map or the demand window.
pub const USER_BASE: u64 = 0x0000_0100_0000_0000;

/// Map one 4 KiB frame at `va` for ring 3. Kernel memory stays without the user bit, so a
/// ring-3 touch anywhere else faults.
pub fn map_user(va: u64, pa: u64, writable: bool) -> bool {
    map_user_in(root(), va, pa, writable)
}

/// `map_user`, in the address space whose PML4 is `root`.
pub fn map_user_in(root: u64, va: u64, pa: u64, writable: bool) -> bool {
    match entry_in(root, va & !0xFFF, 1, true) {
        Some(e) => {
            *e = (pa & ADDR) | PRESENT | USER | if writable { WRITE } else { 0 };
            // SAFETY: drops any stale translation of this one page.
            unsafe { asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags)) };
            true
        }
        None => false,
    }
}

/// A new address space: a PML4 that shares the kernel's mappings (the 1:1 map and the
/// demand window, none of them open to ring 3) and has nothing of its own yet. Ring-3
/// pages added with `map_user_in` are private to it: two spaces can hold different memory
/// at the same virtual address. Returns its PML4's physical address.
pub fn new_space() -> Option<u64> {
    let r = frame::alloc()? as u64;
    let k = table(root());
    let t = table(r);
    for i in 0..512 {
        if k[i] & USER == 0 {
            t[i] = k[i];
        }
    }
    Some(r)
}

/// Give back everything an address space made by `new_space` owns: every ring-3 subtree
/// (the PML4 entries with the user bit, which `new_space` never shares), its pages and
/// its tables, then the PML4 itself. The kernel's shared entries are left alone. The
/// space must not be the current one.
pub fn free_space(root: u64) -> usize {
    fn walk(t: u64, level: u32) -> usize {
        let mut n = 0;
        for i in 0..512 {
            let e = table(t)[i];
            if e & PRESENT == 0 || e & USER == 0 {
                continue;
            }
            let next = e & ADDR;
            if level > 1 && e & HUGE == 0 {
                n += walk(next, level - 1);
            }
            frame::free(next as usize);
            n += 1;
        }
        n
    }
    if root == 0 || root == self::root() {
        return 0;
    }
    let n = walk(root, 4);
    frame::free(root as usize);
    n + 1
}

/// True if `va` is mapped for ring 3 (present and user at every level) in the space `root`.
pub fn user_mapped_in(root: u64, va: u64) -> bool {
    let mut t = root;
    for l in (1..=4).rev() {
        let e = table(t)[index(va, l)];
        if e & PRESENT == 0 || e & USER == 0 {
            return false;
        }
        if l == 1 {
            return true;
        }
        t = e & ADDR;
    }
    false
}

/// Make the address space whose PML4 is `root` the current one.
pub fn switch_to(root: u64) {
    // SAFETY: every space made by `new_space` maps the kernel exactly as the kernel's does.
    unsafe { asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags)) };
}
