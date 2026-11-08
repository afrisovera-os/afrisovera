//! Stand-ins for the two ARM-kernel modules the shared `heap.rs` reaches into.
//!
//! The ARM `frame.rs` is board code (sized from `board::ACTIVE`, clamped by the ARM
//! identity map), so the PC gets its own small frame allocator with the SAME interface
//! `heap.rs` calls: a bitmap over one free region the UEFI memory map reported,
//! chosen above the kernel image so the heap's "arena below the first managed frame"
//! rule holds here as it does on ARM. With it the shared heap grows on the PC exactly
//! as it does on a phone.

pub mod frame {
    use core::sync::atomic::{AtomicUsize, Ordering};

    pub const FRAME_SIZE: usize = 4096;
    /// At most 64 MiB managed: enough for the heap's growth tests, and the bitmap stays
    /// a fixed 2 KiB inside the image.
    const MAX_FRAMES: usize = 16 * 1024;

    static BASE: AtomicUsize = AtomicUsize::new(0);
    static TOTAL: AtomicUsize = AtomicUsize::new(0);
    static USED: AtomicUsize = AtomicUsize::new(0);
    static mut BITMAP: [u64; MAX_FRAMES / 64] = [0; MAX_FRAMES / 64];

    /// Manage `frames` frames from `base` (page-aligned, free conventional RAM).
    pub fn init(base: usize, frames: usize) {
        BASE.store(base, Ordering::Relaxed);
        TOTAL.store(frames.min(MAX_FRAMES), Ordering::Relaxed);
    }

    pub fn alloc() -> Option<usize> {
        let total = TOTAL.load(Ordering::Relaxed);
        // SAFETY: single core; the heap's own GROW_LOCK serialises its callers.
        unsafe {
            let bm = &mut *core::ptr::addr_of_mut!(BITMAP);
            for i in 0..total {
                if bm[i / 64] & (1 << (i % 64)) == 0 {
                    bm[i / 64] |= 1 << (i % 64);
                    USED.fetch_add(1, Ordering::Relaxed);
                    let pa = BASE.load(Ordering::Relaxed) + i * FRAME_SIZE;
                    core::ptr::write_bytes(pa as *mut u8, 0, FRAME_SIZE);
                    return Some(pa);
                }
            }
        }
        None
    }

    pub fn free(pa: usize) {
        let base = BASE.load(Ordering::Relaxed);
        if pa < base || pa >= end() || (pa - base) % FRAME_SIZE != 0 {
            return;
        }
        let i = (pa - base) / FRAME_SIZE;
        // SAFETY: single core; `i` is inside the managed range checked above.
        unsafe {
            let bm = &mut *core::ptr::addr_of_mut!(BITMAP);
            if bm[i / 64] & (1 << (i % 64)) != 0 {
                bm[i / 64] &= !(1 << (i % 64));
                USED.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    pub fn base() -> usize {
        BASE.load(Ordering::Relaxed)
    }
    pub fn end() -> usize {
        base() + TOTAL.load(Ordering::Relaxed) * FRAME_SIZE
    }
    pub fn total_frames() -> usize {
        TOTAL.load(Ordering::Relaxed)
    }
    pub fn used_frames() -> usize {
        USED.load(Ordering::Relaxed)
    }

    /// The shell's `frames` view (the ARM `frame::describe` counterpart).
    pub fn describe() {
        crate::kprintln!(
            "frames: PC frame pool {:#x}..{:#x}: {} frames ({} MiB), {} used, {} free",
            base(),
            end(),
            total_frames(),
            total_frames() * FRAME_SIZE / (1024 * 1024),
            used_frames(),
            total_frames() - used_frames()
        );
    }

    /// The shell's `frames test`: allocate three frames, check each is distinct, aligned,
    /// inside the pool and zeroed, free them, and check the count returns.
    pub fn selftest() {
        let before = used_frames();
        let got = [alloc(), alloc(), alloc()];
        let mut ok = true;
        for (i, f) in got.iter().enumerate() {
            match *f {
                Some(pa) => {
                    let inside = pa >= base() && pa < end() && pa % FRAME_SIZE == 0;
                    // SAFETY: a frame just handed out by this pool, identity-mapped.
                    let zero = unsafe { core::ptr::read_volatile(pa as *const u64) } == 0;
                    let distinct = got.iter().enumerate().all(|(j, g)| j == i || *g != Some(pa));
                    ok &= inside && zero && distinct;
                }
                None => ok = false,
            }
        }
        for pa in got.iter().flatten() {
            free(*pa);
        }
        ok &= used_frames() == before;
        crate::kprintln!(
            "frames test: 3 frames allocated, aligned, zeroed, distinct, freed: {}",
            if ok { "PASS" } else { "FAIL" }
        );
    }
}

pub mod paging {
    /// UEFI hands over with every byte of RAM identity-mapped, so on the PC the
    /// identity map ends at the top of the address space the heap could ever ask about.
    pub const NORMAL_MAP_END: usize = usize::MAX;

    /// The shell's `paging` view: the PC kernel's own 4-level tables (see `mmu.rs`).
    pub fn describe() {
        crate::kprintln!("paging: x86-64 4-level page tables, root CR3 = {:#x} (the kernel's own)", crate::mmu::cr3());
        crate::kprintln!("paging: RAM mapped 1:1 with 2 MiB pages; demand window at {:#x} ({} MiB)",
            crate::mmu::DEMAND_BASE, crate::mmu::DEMAND_SIZE >> 20);
        crate::kprintln!("paging: {} demand faults served since boot",
            crate::mmu::DEMAND_FAULTS.load(core::sync::atomic::Ordering::Relaxed));
    }

    /// The shell's `paging test`: kernel addresses translate to themselves (the 1:1 map)
    /// and an address outside every mapping does not translate.
    pub fn selftest() {
        let probe = describe as fn() as usize as u64;
        let ident = crate::mmu::translate(probe) == Some(probe);
        let hole = crate::mmu::translate(crate::mmu::DEMAND_BASE + crate::mmu::DEMAND_SIZE - 4096).is_none();
        crate::kprintln!("paging test: kernel text maps 1:1 ({}), an unmapped page stays unmapped ({}): {}",
            if ident { "ok" } else { "WRONG" }, if hole { "ok" } else { "WRONG" },
            if ident && hole { "PASS" } else { "FAIL" });
    }
}
