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
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const PRESENT: u64 = 1 << 0;
const WRITE: u64 = 1 << 1;
const USER: u64 = 1 << 2; // ring 3 may use it (needed at every level of the walk)
const PWT: u64 = 1 << 3;
const PCD: u64 = 1 << 4;
const HUGE: u64 = 1 << 7; // in a PD entry: a 2 MiB page
const ADDR: u64 = 0x000F_FFFF_FFFF_F000;
const PAGE_2M: u64 = 2 << 20;
/// No-execute (bit 63): an instruction fetch from the page faults. Only valid once EFER.NXE is
/// set; before that the bit is reserved, so it is only ever written when `nx()` is true.
const NX: u64 = 1 << 63;
const EFER: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;

static NX_ON: AtomicBool = AtomicBool::new(false);

/// True when core 0 switched no-execute on (`init`): ring-3 stack and data pages are then
/// marked NX, so code written there cannot be run.
pub fn nx() -> bool {
    NX_ON.load(Ordering::Relaxed)
}

/// Switch no-execute on for THIS core if the CPU has it (CPUID 8000_0001h EDX bit 20; Intel
/// calls it XD and a BIOS setting can hide it). Core 0 from `init`, every other core as it
/// starts, before it walks a table holding an NX bit. Returns whether it is on here.
pub fn nx_here() -> bool {
    use core::arch::x86_64::__cpuid;
    // SAFETY: CPUID exists on every x86-64 CPU; leaf 8000_0001h is asked only if offered.
    #[allow(unused_unsafe)]
    let has = unsafe { __cpuid(0x8000_0000).eax >= 0x8000_0001 && __cpuid(0x8000_0001).edx & (1 << 20) != 0 };
    if has {
        // SAFETY: EFER exists on every x86-64 CPU and NXE is allowed because the CPU has NX;
        // setting it changes nothing for tables that hold no NX bit.
        unsafe {
            let e = crate::cpu::rdmsr(EFER);
            if e & EFER_NXE == 0 {
                crate::cpu::wrmsr(EFER, e | EFER_NXE);
            }
        }
    }
    has
}

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
    NX_ON.store(nx_here(), Ordering::Relaxed);
    pat_init(); // PAT entry 4 = WC on the bootstrap core (the screen's pages use it)
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

/// F8: leave [lo, hi) (4 KiB aligned) NOT PRESENT in the kernel's 1:1 map: each 2 MiB page that covers it is
/// split into a page table of 4 KiB entries with the same flags and memory type (`split_2m`, PAT bit included), the
/// pages in [lo, hi) are left out, and CR3 is reloaded. Other cores' TLBs are not flushed: it runs at boot, before
/// core 1 starts. Ring-3 spaces copy the kernel's PML4 entries (`new_space`), so they share the guard. False when out
/// of table frames (nothing is changed then) or when [lo, hi) is not under a 2 MiB page of the 1:1 map.
pub fn guard(lo: u64, hi: u64) -> bool {
    let mut big = lo & !(PAGE_2M - 1);
    while big < hi {
        let Some(e) = entry(big, 2, false) else { return false };
        if *e & PRESENT == 0 || *e & HUGE == 0 {
            return false;
        }
        let Some(pt) = split_2m(*e, big) else { return false };
        for (i, slot) in table(pt).iter_mut().enumerate() {
            let pa = big + (i as u64) * 4096;
            if pa >= lo && pa < hi {
                *slot = 0;
            }
        }
        *e = pt | PRESENT | WRITE;
        big += PAGE_2M;
    }
    // SAFETY: reloading CR3 drops every cached translation of the pages just split.
    unsafe { asm!("mov {t}, cr3", "mov cr3, {t}", t = out(reg) _, options(nostack)) };
    true
}

/// Split the 2 MiB page entry `e` (the block at `big`) into a new page table of 512 4 KiB entries with the same
/// P/W/U/PWT/PCD/NX bits, and the PAT bit moved from bit 12 (2 MiB entry) to bit 7 (4 KiB entry), so a split never
/// changes a page's memory type. The one split for `guard` and `map_screen` (architect's D1 condition 2). Returns
/// the table's physical address; the caller points `e` at it. None when out of table frames.
fn split_2m(e: u64, big: u64) -> Option<u64> {
    let flags = e & (PRESENT | WRITE | USER | PWT | PCD | NX) | if e & PAT_2M != 0 { PAT_4K } else { 0 };
    let pt = frame::alloc()? as u64;
    for (i, slot) in table(pt).iter_mut().enumerate() {
        *slot = (big + (i as u64) * 4096) | flags;
    }
    Some(pt)
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
/// ring-3 touch anywhere else faults. A writable page (stack, data) is also no-execute when
/// `nx()`: ring 3 can write code or run it, never both on the same page.
pub fn map_user(va: u64, pa: u64, writable: bool) -> bool {
    map_user_in(root(), va, pa, writable)
}

/// `map_user`, in the address space whose PML4 is `root`.
pub fn map_user_in(root: u64, va: u64, pa: u64, writable: bool) -> bool {
    match entry_in(root, va & !0xFFF, 1, true) {
        Some(e) => {
            *e = (pa & ADDR) | PRESENT | USER | if writable { WRITE | if nx() { NX } else { 0 } } else { 0 };
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

// ---------------------------------------------------------------------------------------
// Write-combining for the screen (11 Nov 2026, the OS lead's D1, architect's plan review):
// PAT entry 4 becomes WC on every core, and only the GOP framebuffer's pages use it.
// ---------------------------------------------------------------------------------------
const PAT_MSR: u32 = 0x277;
/// The PAT bit of a 2 MiB page entry (bit 7 there is the page-size bit).
const PAT_2M: u64 = 1 << 12;
/// The PAT bit of a 4 KiB page entry.
const PAT_4K: u64 = 1 << 7;
/// PAT memory type write-combining, and where entry 4 sits in the MSR.
const WC: u64 = 0x01;
const PAT4_SHIFT: u32 = 32;
static PAT_ON: AtomicBool = AtomicBool::new(false);
/// Cores whose PAT entry 4 read back as WC after `pat_init`.
static PAT_CORES: AtomicU64 = AtomicU64::new(0);
/// The framebuffer range now mapped WC (base, length), and its 4 KiB pages counted.
static WC_BASE: AtomicU64 = AtomicU64::new(0);
static WC_LEN: AtomicU64 = AtomicU64::new(0);
/// Whether the other core answered the last `map_screen`'s shootdown (true when none runs).
static FLUSH_OK: AtomicBool = AtomicBool::new(true);

/// PAT entry 4 reads back as WC on every core now running (architect's D1c): only then is the
/// screen mapped WC, so a core whose PAT write failed never sees the screen's pages as WB.
pub fn pat_all_cores() -> bool {
    PAT_ON.load(Ordering::Relaxed) && PAT_CORES.load(Ordering::Relaxed) >= crate::smp::cores_running()
}

/// Whether the other core answered the last change of the screen's type (D1a).
pub fn others_flushed() -> bool {
    FLUSH_OK.load(Ordering::Relaxed)
}

/// CPUID.1:EDX bit 16: the CPU has the page attribute table.
pub fn pat_present() -> bool {
    // SAFETY: CPUID leaf 1 exists on every x86-64 CPU.
    unsafe { core::arch::x86_64::__cpuid(1).edx & (1 << 16) != 0 }
}

/// On this core: PAT entry 4 = WC, entries 0-3 (the PWT/PCD types every other mapping uses)
/// left at their power-on values. The SDM's sequence for changing memory types: interrupts
/// off, caches off (CR0.CD), WBINVD, write the MSR, WBINVD, flush the TLB, caches back on.
/// Runs on the bootstrap core in `init` and on each further core before it uses the tables.
pub fn pat_init() {
    if !pat_present() {
        return;
    }
    // SAFETY: the documented PAT update sequence; interrupts are restored as they were.
    unsafe {
        let flags: u64;
        asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem));
        let cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        asm!("mov cr0, {}", in(reg) (cr0 | 1 << 30) & !(1u64 << 29), options(nostack)); // CD on, NW off
        asm!("wbinvd", options(nostack));
        let pat = crate::cpu::rdmsr(PAT_MSR);
        crate::cpu::wrmsr(PAT_MSR, (pat & !(0xFF << PAT4_SHIFT)) | WC << PAT4_SHIFT);
        asm!("wbinvd", options(nostack));
        asm!("mov {t}, cr3", "mov cr3, {t}", t = out(reg) _, options(nostack));
        asm!("mov cr0, {}", in(reg) cr0, options(nostack));
        if flags & (1 << 9) != 0 {
            asm!("sti", options(nomem, nostack));
        }
        if (crate::cpu::rdmsr(PAT_MSR) >> PAT4_SHIFT) & 0xFF == WC {
            PAT_ON.store(true, Ordering::Relaxed);
            PAT_CORES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// `sfence`: drain this core's write-combining buffers. Every writer of the screen ends with it
/// (screen::present, screen::clear, the boot trail), so each frame reaches the screen whole
/// (architect's D1 condition 4).
pub fn wc_fence() {
    // SAFETY: a store fence has no other effect.
    unsafe { asm!("sfence", options(nostack, preserves_flags)) };
}

/// Map the GOP framebuffer [pa, pa + len) write-combining (PAT entry 4): 2 MiB pages wholly
/// inside it get the PAT bit, an edge page shared with anything else is split into 4 KiB
/// entries (`split_2m`) and only the framebuffer's get it, so nothing else is ever WC. Without
/// PAT, without WC in every running core's PAT (D1c), or with `wc` false, it is mapped uncached
/// (PWT|PCD, entry 3), as before D1. Returns the 4 KiB pages now WC (0 when uncached); None when
/// out of table frames. Even then the entries already changed are flushed here and on the other
/// core (D1b); `others_flushed` says whether the other core answered (D1a).
///
/// The architect's D1 conditions, as built:
/// 1. A framebuffer below 4 GiB (QEMU's 0x80000000, most laptops) is already in `init`'s 1:1 map
///    as WB 2 MiB pages; `map_mmio` leaves those alone, and the loop below REWRITES them (and
///    any 4 KiB table a split left there). Only pages above the 1:1 map are new.
/// 3. Other cores' TLBs: at boot this runs from screen::attach, before smp::bring_up starts
///    core 1, so no other core holds a translation. At run time (`screen wc on|off`) the caller
///    first checks `smp::can_flush_others` and refuses while another core cannot take an
///    interrupt; after the change this asks every other core by IPI to drain its WC buffers,
///    write back its caches and reload CR3 (`smp::flush_others`), so no core keeps the old type.
/// 4. Before any change of type: sfence and WBINVD on this core (dirty WB lines or full WC
///    buffers must not outlive the old type; `screen wc off` is covered by this). Interrupts are
///    off on this core from the sfence to the CR3 reload (D1b), so the timer's `present` cannot
///    write the screen while its type is half changed, whatever the old type was.
/// 5. Nothing in the kernel reads the framebuffer back (checked by grep, 11 Nov 2026: screen.rs
///    and main.rs's boot trail only write it); a read from WC memory is uncached and slow, so
///    keep it that way.
pub fn map_screen(pa: u64, len: u64, wc: bool) -> Option<u64> {
    if !map_mmio(pa, len) {
        return None; // pages above the 1:1 map get their entries first (uncached)
    }
    let wc = wc && pat_all_cores();
    let (lo, hi) = (pa & !0xFFF, (pa + len + 0xFFF) & !0xFFF);
    // a WC page: the PAT bit set and PWT/PCD clear selects entry 4; uncached: PWT|PCD (entry 3)
    let set = |e: u64, pat: u64| if wc { (e & !(PWT | PCD)) | pat } else { (e & !pat) | PWT | PCD };
    let rewrite = || -> Option<u64> {
        let mut pages = 0;
        let mut big = lo & !(PAGE_2M - 1);
        while big < hi {
            let e = entry(big, 2, false)?;
            if *e & PRESENT == 0 {
                return None;
            }
            let whole = big >= lo && big + PAGE_2M <= hi;
            if *e & HUGE != 0 && whole {
                *e = set(*e, PAT_2M);
                pages += 512;
            } else {
                if *e & HUGE != 0 {
                    let pt = split_2m(*e, big)?; // the edge page: same types, then only the screen's change
                    *e = pt | PRESENT | WRITE;
                }
                let t = table(*e & ADDR);
                for (i, slot) in t.iter_mut().enumerate() {
                    let p = big + (i as u64) * 4096;
                    if p >= lo && p < hi && *slot & PRESENT != 0 {
                        *slot = set(*slot, PAT_4K);
                        pages += 1;
                    }
                }
            }
            big += PAGE_2M;
        }
        Some(pages)
    };
    let flags: u64;
    // SAFETY: saves RFLAGS and masks interrupts on this core until the CR3 reload below (D1b).
    unsafe { asm!("pushfq", "pop {}", "cli", out(reg) flags, options(nomem)) };
    wc_fence();
    // SAFETY: writes back and invalidates this core's caches; no other effect.
    unsafe { asm!("wbinvd", options(nostack)) };
    let done = rewrite();
    // SAFETY: drops every cached translation of the pages changed, also when `rewrite` stopped
    // half way (out of table frames), then restores interrupts as they were.
    unsafe {
        asm!("mov {t}, cr3", "mov cr3, {t}", t = out(reg) _, options(nostack));
        if flags & (1 << 9) != 0 {
            asm!("sti", options(nomem, nostack));
        }
    }
    // condition 3 (true at boot, before core 1 runs); D1a: remember whether core 1 answered
    FLUSH_OK.store(crate::smp::flush_others(), Ordering::Relaxed);
    let pages = done?;
    WC_BASE.store(lo, Ordering::Relaxed);
    WC_LEN.store(hi - lo, Ordering::Relaxed);
    Some(if wc { pages } else { 0 })
}

/// The memory type the kernel's tables give `va` (from its entry's PAT/PCD/PWT bits and this
/// core's PAT MSR): "WB", "WT", "UC-", "UC", "WC", "WP", or "unmapped".
pub fn cache_type(va: u64) -> &'static str {
    let mut t = root();
    let mut l = 4;
    loop {
        let e = table(t)[((va >> (12 + 9 * (l - 1))) & 0x1FF) as usize];
        if e & PRESENT == 0 {
            return "unmapped";
        }
        let leaf = l == 1 || (l == 2 && e & HUGE != 0);
        if leaf {
            let pat = if l == 1 { e & PAT_4K != 0 } else { e & PAT_2M != 0 };
            let idx = (pat as u32) << 2 | ((e & PCD != 0) as u32) << 1 | (e & PWT != 0) as u32;
            // SAFETY: reading this core's PAT MSR (present on every x86-64 CPU in practice;
            // without PAT the power-on table is assumed).
            let msr = if pat_present() { unsafe { crate::cpu::rdmsr(PAT_MSR) } } else { 0x0007_0406_0007_0406 };
            return match (msr >> (8 * idx)) & 0xFF {
                0 => "UC",
                1 => "WC",
                4 => "WT",
                5 => "WP",
                6 => "WB",
                7 => "UC-",
                _ => "?",
            };
        }
        t = e & ADDR;
        l -= 1;
    }
}

/// For `screen`: whether the framebuffer is WC, on how many cores, its range.
pub fn wc_state() -> (bool, u64, u64, u64) {
    (PAT_ON.load(Ordering::Relaxed), PAT_CORES.load(Ordering::Relaxed), WC_BASE.load(Ordering::Relaxed), WC_LEN.load(Ordering::Relaxed))
}
