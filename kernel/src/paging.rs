//! Virtual memory: the first address space.
//!
//! Phase 0.2 uses a single, statically-allocated level-1 translation table with
//! **1 GiB block** entries and a flat identity map — the simplest layout that is
//! still a real MMU configuration. Per-page tables and multiple address spaces
//! (one per process) arrive with userspace in Phase 0.3.
//!
//! **Where the Normal window comes from** (milestone 2.733). Until now this file
//! mapped exactly *one* hard-coded 1 GiB Normal block on every layout — a constant as
//! board-blind as the bitmap's was before 2.732, and after it the tightest limit left:
//! on `board-pine64` the board declares 2 GiB and the allocator managed 1 GiB of it,
//! clamped by a gibibyte that happens to suit QEMU virt. The window is now built from
//! [`crate::board::ACTIVE`]'s own `ram_base`/`ram_size` — as many 1 GiB blocks as the
//! declared range spans — so the map covers the DRAM its board map declares and 2.732's
//! clamp goes slack on every board in the table.
//!
//! QEMU `virt`, the machine we boot, therefore comes out exactly as before:
//!   - `0x0000_0000 .. 0x3FFF_FFFF` → **Device** memory (GIC @ 0x0800_0000,
//!     PL011 UART @ 0x0900_0000, etc.)
//!   - `0x4000_0000 .. 0x7FFF_FFFF` → **Normal** cacheable memory (DRAM: the
//!     kernel image, stacks, heap).
//!
//! Blocks past the last one any board field declares stay **invalid**: a stray access
//! to an address no board map claims must fault, not quietly reach a peripheral.

use crate::cpu;
use core::arch::asm;
use core::ptr::addr_of;

/// A 4 KiB-aligned translation table: 512 × 64-bit descriptors.
#[repr(C, align(4096))]
struct Table {
    entries: [u64; 512],
}

static mut L1: Table = Table { entries: [0; 512] };
// Raspberry Pi (BCM2837) needs 2 MiB granularity for the first GiB (RAM is Normal,
// but the peripherals at 0x3F00_0000 must be Device), so it uses a level-2 table.
#[cfg(feature = "board-raspi")]
static mut L2: Table = Table { entries: [0; 512] };

// F7 (docs/design-f7-stack-guard.md): the 1 GiB block holding the kernel's stack area is split into 2 MiB
// blocks (L2K), and the 2 MiB region holding the stack area into 4 KiB pages (L3S), so the two 64 KiB guard
// ranges below the boot and secondary stacks can be left UNMAPPED. Every other address keeps the attributes
// the 1 GiB block gave it.
#[cfg(not(feature = "board-raspi"))]
static mut L2K: Table = Table { entries: [0; 512] };
#[cfg(not(feature = "board-raspi"))]
static mut L3S: Table = Table { entries: [0; 512] };

extern "C" {
    static __stack_area: u8;
    static __boot_guard_lo: u8;
    static __boot_guard_hi: u8;
    static __sec_guard_lo: u8;
    static __sec_guard_hi: u8;
}

const TWO_MIB: u64 = 0x20_0000;
const FOUR_KIB: u64 = 0x1000;
/// Level-3 **page** descriptor low bits (`0b11`; at level 3 `0b01` is reserved/invalid).
const PAGE_DESC: u64 = 0b11;

/// The two guard ranges `[lo, hi)`: (name, lo, hi). Read from the linker script's symbols.
pub fn guards() -> [(&'static str, usize, usize); 2] {
    [
        ("boot", addr_of!(__boot_guard_lo) as usize, addr_of!(__boot_guard_hi) as usize),
        ("secondary", addr_of!(__sec_guard_lo) as usize, addr_of!(__sec_guard_hi) as usize),
    ]
}

/// Which guard `addr` falls in, if any.
pub fn guard_of(addr: usize) -> Option<&'static str> {
    guards().iter().find(|g| addr >= g.1 && addr < g.2).map(|g| g.0)
}

/// True when the MMU translates `va` for an EL1 read (AT S1E1R; PAR_EL1.F clear). With the MMU off every
/// address "translates", so call this only after `init`.
pub fn translates(va: usize) -> bool {
    let par: u64;
    // SAFETY: address translation probe; changes only PAR_EL1.
    unsafe {
        asm!("at s1e1r, {va}", "isb", "mrs {par}, par_el1", va = in(reg) va, par = out(reg) par, options(nostack));
    }
    par & 1 == 0
}

/// The descriptor the kernel's own L1 holds at `i`. EL0 address spaces copy it for the kernel's DRAM block
/// (process.rs), so they share the guards instead of re-mapping the whole gibibyte as one block.
pub fn kernel_l1_entry(i: usize) -> u64 {
    // SAFETY: L1 is written once in `init` before any process is built; this only reads it.
    unsafe { (*addr_of!(L1)).entries[i] }
}

// --- Descriptor / register bit fields -------------------------------------
const DESC_VALID: u64 = 0b01; // level-1/2 *block* descriptor (bits[1:0]=0b01)
const DESC_AF: u64 = 1 << 10; // Access Flag (fault if 0)
const DESC_SH_INNER: u64 = 0b11 << 8; // Inner shareable
const ATTR_IDX_DEVICE: u64 = 0 << 2; // MAIR index 0
const ATTR_IDX_NORMAL: u64 = 1 << 2; // MAIR index 1

// MAIR_EL1: attr0 = Device-nGnRnE (0x00), attr1 = Normal WB RW-alloc (0xFF).
const MAIR_VALUE: u64 = 0xFF << 8;

// TCR_EL1: 39-bit VA (T0SZ=25 → walk starts at level 1 = 1 GiB blocks), 4 KiB
// granule, inner WB-WA cacheable page-table walks, inner-shareable, 40-bit
// intermediate PA, TTBR1 disabled. A single source of truth so the primary and
// any secondary core program translation identically.
const TCR_VALUE: u64 = 25              // T0SZ
    | (0b01 << 8)                      // IRGN0 = WB-WA
    | (0b01 << 10)                     // ORGN0 = WB-WA
    | (0b11 << 12)                     // SH0   = inner shareable
    | (0b00 << 14)                     // TG0   = 4 KiB
    | (1 << 23)                        // EPD1  = disable TTBR1 walks
    | (0b010 << 32); // IPS   = 40-bit PA

const ONE_GIB: u64 = 1 << 30;

/// Descriptors in one translation table — the whole 512 GiB a 39-bit VA can address at
/// level 1. Nothing may index past it, whatever a board map claims.
pub const ENTRIES: usize = 512;

/// The run of 1 GiB level-1 blocks that carry a board's DRAM.
pub struct Window {
    /// Index of the first block holding DRAM (`ram_base / 1 GiB`).
    pub first: usize,
    /// How many consecutive blocks the declared range spans. Zero when the map
    /// declares no DRAM at all.
    pub blocks: usize,
}

impl Window {
    /// First address the window covers — the base of the block holding `ram_base`.
    pub const fn start(&self) -> usize {
        self.first * (ONE_GIB as usize)
    }

    /// First address past the window — the value the allocator takes as its ceiling.
    pub const fn end(&self) -> usize {
        (self.first + self.blocks) * (ONE_GIB as usize)
    }

    /// Whether level-1 block `i` is inside the Normal window.
    pub const fn contains(&self, i: usize) -> bool {
        i >= self.first && i < self.first + self.blocks
    }
}

/// Which 1 GiB blocks cover `[ram_base, ram_base + ram_size)`.
///
/// A **pure function** of the two figures, for the reason `frame::plan` and
/// `board::resolve_from` are: this machine boots with one board, so every other board's
/// window would otherwise be arithmetic nobody ever runs. `pmap test` puts the whole
/// `board::KNOWN` table through it. Blocks are counted inclusively from the block
/// holding the base to the block holding the last byte, so a range straddling a
/// gibibyte boundary gets both — and the result is clamped to [`ENTRIES`], because a
/// map claiming DRAM past the address space is a bug in the map, not licence to write
/// off the end of the table.
pub const fn window(ram_base: u64, ram_size: u64) -> Window {
    let first = (ram_base / ONE_GIB) as usize;
    if ram_size == 0 || first >= ENTRIES {
        return Window { first: if first < ENTRIES { first } else { ENTRIES }, blocks: 0 };
    }
    let last = (ram_base.saturating_add(ram_size - 1) / ONE_GIB) as usize;
    let last = if last < ENTRIES { last } else { ENTRIES - 1 };
    Window { first, blocks: last - first + 1 }
}

/// The Normal window of the board compiled in — the one the MMU is actually programmed
/// with below, and the one the allocator is bounded by.
pub const NORMAL: Window = window(crate::board::ACTIVE.ram_base, crate::board::ACTIVE.ram_size);

/// First address past the DRAM this identity map covers as **Normal**, cacheable
/// memory — the hard ceiling on any physical allocator, stated by the code that
/// enforces it rather than copied into the code that must respect it (2.731), and
/// since 2.733 derived from the board's declared DRAM rather than from a fixed
/// gibibyte: `[1 GiB, 2 GiB)` on QEMU virt (256 MiB inside block 1), `[0, 1 GiB)` on
/// the Pi, `[1 GiB, 3 GiB)` on PINE64's declared 2 GiB. A frame handed out past this
/// line is not slow or uncached — it is a translation fault, which is why the
/// allocator treats it as a bound and not as a preference.
pub const NORMAL_MAP_END: usize = NORMAL.end();

/// How many level-1 blocks `init` writes at all: the Normal window, extended to reach
/// every MMIO window this board's map declares (the Pi 4's peripherals sit at
/// `0xFE00_0000`, three blocks above its DRAM). Everything below the top that is not
/// DRAM is Device; everything above stays an invalid descriptor.
const fn mapped_blocks() -> usize {
    let m = crate::board::ACTIVE;
    let mmio = [
        m.uart as u64,
        m.gic_dist as u64,
        m.gic_cpu as u64,
        m.rtc as u64,
        m.gpio as u64,
        m.fwcfg as u64,
        m.virtio,
    ];
    // Block 0 is always mapped: every board we know puts something in the low
    // gibibyte, and a map with neither DRAM nor MMIO there is not a board.
    let mut n = if NORMAL.first + NORMAL.blocks > 1 { NORMAL.first + NORMAL.blocks } else { 1 };
    let mut i = 0;
    while i < mmio.len() {
        // 0 means "this board has no such device" (see `board::Map`), not address 0.
        if mmio[i] != 0 {
            let top = (mmio[i] / ONE_GIB) as usize + 1;
            if top > n {
                n = top;
            }
        }
        i += 1;
    }
    if n > ENTRIES {
        ENTRIES
    } else {
        n
    }
}

/// Level-1 blocks `init` writes. Public so `pmap` can state where mapping stops.
pub const MAPPED_BLOCKS: usize = mapped_blocks();

// The Pi's fine-grained path below splits the FIRST block with a level-2 table, so it
// is only correct while the Pi's declared DRAM is that one block. If a future map says
// otherwise, this build must not silently map the rest of it as Device.
#[cfg(feature = "board-raspi")]
const _: () = assert!(
    NORMAL.first == 0 && NORMAL.blocks == 1,
    "board-raspi declares DRAM outside level-1 block 0; the L2 split covers only that block"
);

/// Build the identity map and turn the MMU on.
pub fn init() {
    // SAFETY: single-threaded, runs once at boot before any other core/IRQ.
    unsafe {
        let l1 = &mut *core::ptr::addr_of_mut!(L1);

        // --- QEMU virt (default): 1 GiB blocks, Normal across the board's declared
        //     DRAM and Device over everything else it declares (2.733). On virt that
        //     is the same two entries as before — block 0 Device (GIC, PL011, virtio),
        //     block 1 Normal (DRAM at 0x4000_0000) — because the board map says so and
        //     no longer because the block index is written here. ---
        #[cfg(not(feature = "board-raspi"))]
        {
            for i in 0..MAPPED_BLOCKS {
                let pa = (i as u64) * ONE_GIB;
                l1.entries[i] = if NORMAL.contains(i) {
                    pa | DESC_AF | DESC_SH_INNER | ATTR_IDX_NORMAL | DESC_VALID
                } else {
                    pa | DESC_AF | ATTR_IDX_DEVICE | DESC_VALID
                };
            }

            // F7: punch the two stack guards out of the Normal window. The stack area is 2 MiB aligned and
            // fits one 2 MiB region (linker ASSERT), so one L2 + one L3 table cover it.
            let area = addr_of!(__stack_area) as u64;
            let blk = (area / ONE_GIB) as usize;
            if NORMAL.contains(blk) && blk < MAPPED_BLOCKS {
                // Every new entry keeps EXACTLY the attributes the 1 GiB block gave (architect condition 1):
                // they are taken from that block's own descriptor, not restated here. Only the output address
                // and the descriptor type change.
                const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;
                let normal = l1.entries[blk] & !ADDR_MASK & !0b11;
                let l2 = &mut *core::ptr::addr_of_mut!(L2K);
                let l3 = &mut *core::ptr::addr_of_mut!(L3S);
                let gib = (blk as u64) * ONE_GIB;
                for i in 0..512 {
                    l2.entries[i] = (gib + (i as u64) * TWO_MIB) | normal | DESC_VALID; // 2 MiB block
                }
                for p in 0..512 {
                    let pa = area + (p as u64) * FOUR_KIB;
                    l3.entries[p] = if guard_of(pa as usize).is_some() {
                        0 // invalid: any access faults
                    } else {
                        pa | normal | PAGE_DESC
                    };
                }
                l2.entries[((area - gib) / TWO_MIB) as usize] = (addr_of!(L3S) as u64) | TABLE_DESC;
                l1.entries[blk] = (addr_of!(L2K) as u64) | TABLE_DESC;
            }
        }

        // --- Raspberry Pi (BCM2837): first GiB is RAM (Normal) except the
        //     peripheral window at 0x3F00_0000..0x3FFF_FFFF (Device). This is the one
        //     board whose DRAM block needs 2 MiB granularity, so it keeps its own path
        //     rather than the block loop above; the const assert on `NORMAL` holds it
        //     to the single block that path actually covers. ---
        #[cfg(feature = "board-raspi")]
        {
            const TWO_MIB: u64 = 0x20_0000;
            const PERIPH_BLOCK: usize = (0x3F00_0000 / 0x20_0000) as usize; // = 504
            let l2 = &mut *core::ptr::addr_of_mut!(L2);
            for i in 0..512 {
                let pa = (i as u64) * TWO_MIB;
                l2.entries[i] = if i >= PERIPH_BLOCK {
                    pa | DESC_AF | ATTR_IDX_DEVICE | DESC_VALID // peripherals
                } else {
                    pa | DESC_AF | DESC_SH_INNER | ATTR_IDX_NORMAL | DESC_VALID // RAM
                };
            }
            // L1[0] points at the L2 table (fine-grained RAM/peripheral split).
            l1.entries[0] = (core::ptr::addr_of!(L2) as u64) | TABLE_DESC;
            // L1[1]: the BCM2836/2837 LOCAL peripherals (per-core timer + local
            // interrupt controller) live at 0x4000_0000. Map [1 GiB, 2 GiB) as a
            // Device block so the local timer-IRQ registers are reachable.
            l1.entries[1] = (1 * ONE_GIB) | DESC_AF | ATTR_IDX_DEVICE | DESC_VALID;
        }

        let ttbr0 = addr_of!(L1) as u64;
        let tcr: u64 = TCR_VALUE;

        asm!(
            "msr mair_el1, {mair}",
            "msr tcr_el1,  {tcr}",
            "msr ttbr0_el1,{ttbr0}",
            "isb",
            // Invalidate all EL1 TLB entries before switching on translation.
            "tlbi vmalle1",
            "dsb sy",
            "isb",
            mair = in(reg) MAIR_VALUE,
            tcr = in(reg) tcr,
            ttbr0 = in(reg) ttbr0,
            options(nostack),
        );

        // Enable MMU (M) + data cache (C) + instruction cache (I) in SCTLR_EL1.
        let mut sctlr: u64;
        asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack));
        sctlr |= 1 << 0; // M — MMU enable
        sctlr |= 1 << 2; // C — data cache
        sctlr |= 1 << 12; // I — instruction cache
        cpu::dsb();
        asm!("msr sctlr_el1, {}", in(reg) sctlr, options(nostack));
        cpu::isb();
    }
}

/// Enable the MMU on a **secondary** core using the identity map the primary
/// already built (same TTBR0 = the shared L1 table). This does NOT rebuild the
/// table — it only programs this core's MAIR/TCR/TTBR0 to match the primary and
/// turns translation on, so the core stays running (the identity map means
/// VA==PA, so its PC, SP, and stack remain valid across the switch). It must run
/// before the core touches shared memory with exclusives (spinlocks) or
/// cacheable atomics, which are only well-defined with the MMU on. QEMU virt
/// only; the Pi build never brings up a secondary.
#[cfg(not(feature = "board-raspi"))]
pub fn enable_mmu_secondary() {
    // SAFETY: the primary finished building L1 (with a dsb) before powering this
    // core on; page-table walks are inner-shareable cacheable, so the table is
    // coherent across cores. We program the identical translation config.
    unsafe {
        let ttbr0 = addr_of!(L1) as u64;
        asm!(
            "msr mair_el1, {mair}",
            "msr tcr_el1,  {tcr}",
            "msr ttbr0_el1,{ttbr0}",
            "isb",
            "tlbi vmalle1",
            "dsb sy",
            "isb",
            mair = in(reg) MAIR_VALUE,
            tcr = in(reg) TCR_VALUE,
            ttbr0 = in(reg) ttbr0,
            options(nostack),
        );
        let mut sctlr: u64;
        asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack));
        sctlr |= 1 << 0; // M — MMU enable
        sctlr |= 1 << 2; // C — data cache
        sctlr |= 1 << 12; // I — instruction cache
        cpu::dsb();
        asm!("msr sctlr_el1, {}", in(reg) sctlr, options(nostack));
        cpu::isb();
    }
}

// --- Primitives reused when building per-process (userspace) address spaces ---

/// Descriptor low bits selecting a level-1/2 **block** (`0b01`).
pub const BLOCK_DESC: u64 = DESC_VALID;
/// Descriptor low bits selecting a **table** pointer (`0b11`).
pub const TABLE_DESC: u64 = 0b11;
/// Access Flag bit.
pub const AF: u64 = DESC_AF;
/// Inner-shareable bits.
pub const SH_INNER: u64 = DESC_SH_INNER;
/// MAIR index for Normal cacheable memory.
pub const ATTR_NORMAL: u64 = ATTR_IDX_NORMAL;

/// Physical base of the kernel's own L1 table (TTBR0 value for kernel/idle).
pub fn kernel_ttbr0() -> u64 {
    addr_of!(L1) as u64
}

/// EL1-only Device block descriptor for physical address `pa`.
pub fn device_block(pa: u64) -> u64 {
    pa | DESC_AF | ATTR_IDX_DEVICE | DESC_VALID
}

/// EL1-only Normal-memory block descriptor for physical address `pa`.
pub fn normal_block_el1(pa: u64) -> u64 {
    pa | DESC_AF | DESC_SH_INNER | ATTR_IDX_NORMAL | DESC_VALID
}

// --- What the map is, and why (`pmap`, `pmap test`) ------------------------

/// The identity map as built for this board — printed by `pmap`, so the ceiling the
/// allocator reports has a visible cause rather than being a number from another file.
pub fn describe() {
    let m = crate::board::ACTIVE;
    crate::kprintln!(
        "  identity map   Normal {:#010x}..{:#010x} ({} x 1GiB block{}, from board DRAM {} MiB @ {:#010x})",
        NORMAL.start(),
        NORMAL_MAP_END,
        NORMAL.blocks,
        if NORMAL.blocks == 1 { "" } else { "s" },
        m.ram_size / (1024 * 1024),
        m.ram_base
    );
    crate::kprintln!(
        "                 Device elsewhere below {:#010x}; above that: unmapped (faults)",
        MAPPED_BLOCKS * (ONE_GIB as usize)
    );
    #[cfg(not(feature = "board-raspi"))]
    for (name, lo, hi) in guards() {
        crate::kprintln!(
            "  stack guard    {} {:#010x}..{:#010x} ({} KiB, unmapped 4 KiB pages)",
            name,
            lo,
            hi,
            (hi - lo) / 1024
        );
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

/// Put every board in `board::KNOWN` through [`window`] (`pmap test`).
///
/// The point of 2.733 is a claim about boards this machine is not: that the Normal
/// window follows each map's declared DRAM, and therefore that 2.732's clamp — the
/// bitmap held to `NORMAL_MAP_END` — no longer binds on any of them. [`window`] is
/// pure, so that claim is checkable here instead of on four machines.
pub fn selftest() {
    const GIB: u64 = ONE_GIB;
    let (mut pass, mut total) = (0u32, 0u32);
    crate::kprintln!("identity map: Normal-window self-test over every known board");

    // The claim itself, board by board: the window covers the DRAM the map declares,
    // so `min(declared, window)` — the bitmap's size since 2.732 — is `declared`.
    for (name, _, m) in crate::board::KNOWN.iter() {
        let w = window(m.ram_base, m.ram_size);
        let covered = (w.end() as u64).saturating_sub(m.ram_base) >= m.ram_size;
        let starts_at_ram = w.first == (m.ram_base / GIB) as usize;
        total += 1;
        if covered && starts_at_ram {
            pass += 1;
            crate::kprintln!(
                "  [ok]   board {}: {} MiB declared, window {:#x}..{:#x} ({} x 1GiB) — clamp slack",
                name, m.ram_size / (1024 * 1024), w.start(), w.end(), w.blocks
            );
        } else {
            crate::kprintln!(
                "  [FAIL] board {}: {} MiB declared, window {:#x}..{:#x} does not cover it",
                name, m.ram_size / (1024 * 1024), w.start(), w.end()
            );
        }
    }

    // The window is a run of blocks, not a byte range: it starts at a block boundary at
    // or below the DRAM base and ends at one at or above the top.
    let w = window(0x4000_0000, 256 * 1024 * 1024);
    case_ok(&mut pass, &mut total,
        w.first == 1 && w.blocks == 1 && w.end() == 0x8000_0000,
        "256 MiB at 1 GiB is one block: the whole gibibyte that holds it");

    // PINE64's case, the one 2.732's clamp was binding on.
    let w = window(0x4000_0000, 2 * GIB);
    case_ok(&mut pass, &mut total,
        w.first == 1 && w.blocks == 2 && w.end() == 0xC000_0000,
        "2 GiB at 1 GiB spans TWO blocks, so a declared gibibyte is not clamped away");

    // A range straddling a boundary needs both blocks — the off-by-one that would
    // unmap the last page of DRAM on a board whose memory is not gibibyte-aligned.
    let w = window(GIB - 4096, 8192);
    case_ok(&mut pass, &mut total, w.first == 0 && w.blocks == 2,
        "a range straddling a 1 GiB boundary gets both blocks, not just the first");

    let w = window(0, GIB);
    case_ok(&mut pass, &mut total, w.first == 0 && w.blocks == 1 && w.end() == GIB as usize,
        "exactly one gibibyte from zero is one block, not two");

    // A map with no DRAM to declare (`ram_size == 0`) maps no Normal memory at all,
    // rather than a block of whatever sits at address 0.
    let w = window(0x4000_0000, 0);
    case_ok(&mut pass, &mut total, w.blocks == 0 && w.end() <= 0x8000_0000,
        "a map declaring no DRAM gets no Normal blocks");

    // The table has 512 entries and the walk starts at level 1; a map claiming DRAM
    // past the address space must not write off the end of it.
    let w = window(511 * GIB, 8 * GIB);
    case_ok(&mut pass, &mut total, w.first + w.blocks <= ENTRIES,
        "a map claiming DRAM past the address space is clamped to the table");
    let w = window(600 * GIB, GIB);
    case_ok(&mut pass, &mut total, w.first + w.blocks <= ENTRIES && w.blocks == 0,
        "a DRAM base past the address space maps nothing at all");

    // And the live map is that same arithmetic, not a constant beside it.
    case_ok(&mut pass, &mut total,
        NORMAL_MAP_END == NORMAL.end()
            && NORMAL.end() == window(crate::board::ACTIVE.ram_base,
                                      crate::board::ACTIVE.ram_size).end(),
        "this board's ceiling IS window(board::ACTIVE), not a constant beside it");
    case_ok(&mut pass, &mut total,
        MAPPED_BLOCKS >= NORMAL.first + NORMAL.blocks
            && (crate::board::ACTIVE.uart as u64) / GIB < MAPPED_BLOCKS as u64,
        "mapping reaches past the Normal window far enough to include this board's UART");

    crate::kprintln!(
        "identity map: {}/{} window checks held (Normal ends at {:#010x}, {} block(s))",
        pass, total, NORMAL_MAP_END, NORMAL.blocks
    );
}
