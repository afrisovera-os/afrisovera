//! Mansa on a PC: the x86-64 kernel (PC track, step 1; owner's decision, 3 Nov 2026).
//!
//! A UEFI application: the firmware of every modern PC (and QEMU's `q35` with OVMF)
//! loads `EFI/BOOT/BOOTX64.EFI` from a FAT partition (a USB stick on real hardware)
//! and jumps here. This is the PC counterpart of `main.rs`'s ARM boot path:
//!
//!   1. take the console (COM1) and the screen (UEFI GOP framebuffer);
//!   2. read the firmware's memory map, then EXIT boot services; from that line on
//!      the firmware is gone and the kernel owns the machine;
//!   3. install the kernel's own GDT and IDT, and prove the trap path with a breakpoint;
//!   4. bring up the local APIC and take timer interrupts;
//!   5. run SHARED kernel code: `heap.rs` (as the global allocator, plus its own
//!      self-test), `sha256.rs`, `sha512.rs`, `ed25519.rs`, `font.rs`, the very same
//!      files the ARM kernel builds, included by path, not copied.
//!
//! Not yet (later PC-track steps, docs/05): the kernel's own page tables, ring-3
//! processes + syscalls, capabilities + IPC, the scheduler, PCI virtio, real drivers.
//!
//! Build: cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi
//! Test:  scripts/pc-boot-test.sh
#![no_std]
#![no_main]

#[cfg(not(target_arch = "x86_64"))]
compile_error!("mansa-pc is the x86-64 PC kernel: build it with --target x86_64-unknown-uefi");

extern crate alloc;

#[macro_use]
mod uart;
mod cpu;
mod mmu;
mod proc;
mod uefi;
mod user;
mod qemu;
mod shims;
mod smp;
mod stackinfo;
use shims::{frame, paging};

// Shared with the ARM kernel, unchanged.
#[allow(dead_code)]
#[path = "../heap.rs"]
mod heap;
#[allow(dead_code)]
#[path = "../sha256.rs"]
mod sha256;
#[allow(dead_code)]
#[path = "../drbg.rs"]
mod drbg;
#[allow(dead_code)]
#[path = "../drbg_kat.rs"]
mod drbg_kat;
#[allow(dead_code)]
#[path = "../sha512.rs"]
mod sha512;
#[allow(dead_code)]
#[path = "../ed25519.rs"]
mod ed25519;
#[allow(dead_code)]
#[path = "../font.rs"]
mod font;

// PC track step 2 (docs/05): the ARM kernel's interactive shell, AfriSh, and the
// arch-neutral modules behind it: the SAME source files, included by path.
#[allow(dead_code)]
#[path = "../shell.rs"]
mod shell;
#[allow(dead_code)]
#[path = "../asili.rs"]
mod asili;
#[allow(dead_code)]
#[path = "../config.rs"]
mod config;
#[allow(dead_code)]
#[path = "../daraja.rs"]
mod daraja;
#[allow(dead_code)]
#[path = "../fmtbuf.rs"]
mod fmtbuf;
#[allow(dead_code)]
#[path = "../framebuffer.rs"]
mod framebuffer;
#[allow(dead_code)]
#[path = "../integrity.rs"]
mod integrity;
#[allow(dead_code)]
#[path = "../nia.rs"]
mod nia;
#[path = "../appget.rs"]
mod appget;
#[allow(dead_code)]
#[path = "../store.rs"]
mod store;
#[allow(dead_code)]
#[path = "../tonga.rs"]
mod tonga;

#[path = "../afriui/desk.rs"] // the Sovereign UI (docs/design/afriui.md); the phone builds it too
mod desk;
#[allow(dead_code)]
#[path = "../uart_ns16550.rs"]
mod uart_ns16550;
#[allow(dead_code)]
#[path = "../vfs.rs"]
mod vfs;
#[allow(dead_code)]
#[path = "../voice.rs"]
mod voice;
#[allow(dead_code)]
#[path = "../web.rs"]
mod web;

// PC stand-ins for the ARM kernel's hardware modules, under the names the shared code
// uses (crate::board, crate::virtio, ...): the PC's real device where one is cheap (TSC +
// PIT timer, CMOS clock, RDRAND, UEFI reset), an honest "absent" otherwise. See compat/.
#[path = "compat/board.rs"]
mod board;
#[path = "compat/cmos.rs"]
mod cmos;
#[path = "compat/fdt.rs"]
mod fdt;
#[path = "compat/fwcfg.rs"]
mod fwcfg;
#[path = "compat/gic.rs"]
mod gic;
#[path = "compat/gpio.rs"]
mod gpio;
#[path = "compat/process.rs"]
mod process;
#[path = "compat/psci.rs"]
mod psci;
#[path = "compat/rng.rs"]
mod rng;
#[path = "compat/sched.rs"]
mod sched;
#[path = "compat/shell_pc.rs"]
mod shell_pc;
#[path = "compat/timer.rs"]
mod timer;
// the ARM driver itself; its transport underneath is PCI (virtio.rs -> pc/virtio_pci.rs)
#[path = "../virtio.rs"]
mod virtio_core;
// `crate::virtio` = virtio_core, its disk calls widened to the PC's SATA disks (disk.rs)
#[path = "disk.rs"]
mod virtio;
mod pci;
mod ahci;
mod ide;
mod nvme;
mod e1000;
mod rtl8169;
mod nic;
mod xhci;
mod ps2;
mod screen;
mod canvas; // the desktop at the screen's own resolution (docs/design/afriui.md section 13)
mod conback;
mod helpview;
mod font_lower;
mod acpi;
mod gpt;
mod bootent;
mod esp;
mod fwinfo;
mod wifi;
mod hda;
mod sdhci;
mod report;

use alloc::vec::Vec;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};

/// UEFI's runtime-services table, kept from boot: `psci::poweroff`/`reboot` reset through it.
static RUNTIME_SERVICES: AtomicUsize = AtomicUsize::new(0);
/// The RAM the firmware's memory map reported as usable, in MiB (field item F11): the
/// machine's real memory, not the kernel's fixed frame pool. Shown by the desktop and `mem`.
static RAM_MIB: AtomicUsize = AtomicUsize::new(0);

/// The machine's RAM in MiB (the `crate::ram_mib` the shared desktop and `mem` print). On a
/// PC this is what the firmware reported; 0 until the memory map has been read.
pub fn ram_mib() -> u64 {
    RAM_MIB.load(Ordering::Relaxed) as u64
}

pub fn runtime_services() -> *const uefi::RuntimeServices {
    RUNTIME_SERVICES.load(Ordering::Relaxed) as *const uefi::RuntimeServices
}

/// Cores running kernel code (the ARM `cores_online` the shell prints): the PC kernel does
/// not start the application processors yet.
pub fn cores_online() -> u32 {
    1
}

/// The ARM second core's affinity id; the PC starts no second core.
pub fn secondary_aff0() -> Option<u32> {
    None
}

/// The firmware's memory map is copied here: a buffer inside the kernel image, so
/// reading the map never asks the firmware for memory (which would change the map).
static mut MAP_BUF: [u8; 64 * 1024] = [0; 64 * 1024];

/// The frames the shared heap grows into, until the PC has its own page tables and can
/// hand out the free RAM the memory map reports (that RAM still holds the firmware's page
/// tables, which CR3 points at).
///
/// Declared in assembler, not as a Rust static, for two reasons measured on 3 Nov 2026:
/// a Rust static with an explicit `#[link_section]` is emitted as INITIALISED data (the
/// .efi grew by 16 MiB), and without one the linker placed it BELOW the heap's arena,
/// which the shared heap self-test rejects ("the REAL arena ... lies below the first
/// managed frame"). The `"bw"` flags make `.bss$zz` uninitialised (reserved by the
/// loader, absent from the file), and the PE linker places it after the image's data.
/// Still CHECKED at boot (the frame-pool line and the heap self-test), never assumed.
const POOL_FRAMES: usize = 16 * 1024; // 64 MiB: the allocator's whole range (shims.rs MAX_FRAMES), so the heap's 1/4 share can serve the registry's growth asks
core::arch::global_asm!(
    ".section .bss$zz,\"bw\"",
    ".balign 4096",
    ".global mansa_pc_frame_pool",
    "mansa_pc_frame_pool:",
    ".space 67108864", // POOL_FRAMES * 4096
    ".text",
);
extern "C" {
    static mansa_pc_frame_pool: u8;
}

struct Screen {
    base: *mut u32,
    w: usize,
    h: usize,
    stride: usize,
    bgr: bool,
}

impl Screen {
    fn px(&self, x: usize, y: usize, rgb: u32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let v = if self.bgr {
            rgb // BGR-reserved in memory = 0x00RRGGBB as a little-endian u32
        } else {
            ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
        };
        // SAFETY: (x, y) is inside the mode the firmware reported.
        unsafe { core::ptr::write_volatile(self.base.add(y * self.stride + x), v) };
    }
    fn fill(&self, x0: usize, y0: usize, w: usize, h: usize, rgb: u32) {
        for y in y0..(y0 + h).min(self.h) {
            for x in x0..(x0 + w).min(self.w) {
                self.px(x, y, rgb);
            }
        }
    }
    fn text(&self, x0: usize, y0: usize, s: &str, rgb: u32, scale: usize) {
        for (i, &c) in s.as_bytes().iter().enumerate() {
            let g = font::glyph(c);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..8 {
                    if (bits >> (7 - col)) & 1 == 1 {
                        let x = x0 + (i * 8 + col) * scale;
                        self.fill(x, y0 + row * scale, scale, scale, rgb);
                    }
                }
            }
        }
    }
}

// ---- The boot trail (8 Nov 2026) ------------------------------------------------------
// The owner's first real-PC try (an HP ENVY laptop, i5-6200U) showed a blank screen and
// nothing else: a laptop has no COM1, and the screen showed nothing until the shell. So
// every boot line is also drawn straight onto the firmware's framebuffer from the first
// moment, below the title, until the shell takes the screen: if a real machine stops
// part-way, a photo of the screen says where.
static mut EARLY: Option<Screen> = None;
static mut EARLY_ROW: usize = 0;
static mut EARLY_COL: usize = 0;
static EARLY_BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
const EARLY_TOP: usize = 160;
const EARLY_BG: u32 = 0x10_2018;

fn early_scale(s: &Screen) -> usize {
    if s.w >= 1600 { 2 } else { 1 }
}

/// Called from every print (uart.rs Tee) while the boot trail is on.
pub fn early_write(text: &str) {
    if EARLY_BUSY.swap(true, Ordering::Acquire) {
        return; // a fault inside the trail itself: never recurse
    }
    // SAFETY: boot runs on one core; prints hold PRINT_LOCK; EARLY is only changed by boot.
    unsafe {
        if let Some(s) = (*core::ptr::addr_of!(EARLY)).as_ref() {
            let sc = early_scale(s);
            let (cw, ch) = (8 * sc, 10 * sc);
            let cols = (s.w.saturating_sub(48)) / cw;
            let rows = (s.h.saturating_sub(EARLY_TOP + 8)) / ch;
            for b in text.bytes() {
                if b == b'\n' || EARLY_COL >= cols {
                    EARLY_COL = 0;
                    EARLY_ROW += 1;
                    if EARLY_ROW >= rows {
                        // full: start again at the top (the newest lines are the ones that matter)
                        EARLY_ROW = 0;
                    }
                    let y = EARLY_TOP + EARLY_ROW * ch;
                    s.fill(0, y, s.w, ch * 2, EARLY_BG);
                    if b == b'\n' {
                        continue;
                    }
                }
                if b == b'\r' {
                    continue;
                }
                let c = if (0x20..0x7F).contains(&b) { b } else { b'?' };
                let mut one = [0u8; 4];
                let st = (c as char).encode_utf8(&mut one);
                s.text(24 + EARLY_COL * cw, EARLY_TOP + EARLY_ROW * ch, st, 0xD0_D8D0, sc);
                EARLY_COL += 1;
            }
        }
    }
    mmu::wc_fence(); // the screen may be write-combining by now (D1): drain this core's buffers
    EARLY_BUSY.store(false, Ordering::Release);
}

/// Begin the boot trail on this screen.
fn early_on(s: &Screen) {
    // SAFETY: boot, single core, before anything prints through it.
    unsafe {
        EARLY = Some(Screen { base: s.base, w: s.w, h: s.h, stride: s.stride, bgr: s.bgr });
        EARLY_ROW = 0;
        EARLY_COL = 0;
    }
}

/// Carry the boot trail on where it was (after the kernel's own page tables are in).
fn early_resume(s: &Screen) {
    // SAFETY: boot, single core.
    unsafe { EARLY = Some(Screen { base: s.base, w: s.w, h: s.h, stride: s.stride, bgr: s.bgr }) };
}

/// End the boot trail (the shell takes the screen); `clear` wipes what it drew.
fn early_off(clear: bool) {
    while EARLY_BUSY.swap(true, Ordering::Acquire) {
        core::hint::spin_loop(); // a print (another core's) is drawing: let it finish
    }
    // SAFETY: EARLY_BUSY is held, so no print is drawing through EARLY.
    let s = unsafe { (*core::ptr::addr_of_mut!(EARLY)).take() };
    if let (true, Some(s)) = (clear, s) {
        s.fill(0, EARLY_TOP, s.w, s.h - EARLY_TOP.min(s.h), EARLY_BG);
    }
    mmu::wc_fence();
    EARLY_BUSY.store(false, Ordering::Release);
}

fn screen(u: &uefi::Uefi) -> Option<Screen> {
    let gop = u.graphics()?;
    // SAFETY: the firmware's GOP mode and mode-info structures.
    let (mode, info) = unsafe { (&*gop.mode, &*(*gop.mode).info) };
    if info.pixel_format > 1 {
        crate::kprintln!("[pc] GOP: pixel format {} not drawn (bitmask/blt-only)", info.pixel_format);
        return None;
    }
    Some(Screen {
        base: mode.fb_base as *mut u32,
        w: info.hres as usize,
        h: info.vres as usize,
        stride: info.stride as usize,
        bgr: info.pixel_format == 1,
    })
}

#[no_mangle]
extern "efiapi" fn efi_main(image: uefi::Handle, st: *mut uefi::SystemTable) -> uefi::Status {
    // F8 measure-first: the PC runs on the firmware's stack; remember where it started.
    stackinfo::note_entry();
    uart::init();
    let u = uefi::Uefi { image, st };
    // The screen FIRST, before anything that could stop on a real machine: the title, then
    // every boot line below it (the boot trail), so a photo of a stuck PC says where.
    let scr = screen(&u);
    if let Some(s) = &scr {
        s.fill(0, 0, s.w, s.h, 0x10_2018);
        s.fill(0, 0, s.w, 6, 0xE0_A030);
        s.text(24, 32, "AfriSovera OS", 0xFF_FFFF, 3);
        s.text(24, 70, "Mansa kernel on a PC (x86-64)", 0xE0_A030, 2);
        early_on(s);
    }
    kprintln!();
    kprintln!("[pc] Mansa x86-64 (PC), started by UEFI firmware");
    if !uart::present() {
        kprintln!("[pc] no COM1 serial port (a laptop): the screen is the console");
    }
    // SAFETY: the system table the firmware passed in; the vendor is a NUL-ended UTF-16 string.
    unsafe {
        let mut p = (*st).firmware_vendor;
        kprint!("[pc] firmware: ");
        while !p.is_null() && *p != 0 {
            let c = *p;
            kprint!("{}", if c < 0x80 { c as u8 as char } else { '?' });
            p = p.add(1);
        }
        kprintln!(" (revision {:#x})", (*st).firmware_revision);
    }
    u.disable_watchdog();
    // SAFETY: the system table is valid; runtime services survive exit_boot_services.
    let rt = unsafe { (*st).runtime_services } as *const uefi::RuntimeServices;
    RUNTIME_SERVICES.store(rt as usize, Ordering::Relaxed);

    // The screen, while the firmware can still tell us where it is.
    // ACPI: the cores and I/O APICs of the machine (read while firmware tables are certain)
    // SAFETY: the system table is valid until exit_boot_services.
    unsafe { acpi::init((*st).n_config, (*st).config_table as u64) };
    // the boot file itself, for installdisk (it cannot be read once the firmware is gone)
    u.keep_own_image();
    // the firmware's boot entries, for bootentries (read only; the firmware is not asked again)
    u.keep_boot_entries();
    // the firmware's Secure Boot state and version, for fwinfo (read only)
    // SAFETY: the system table the firmware passed in, before exit_boot_services.
    unsafe { fwinfo::keep(st) };
    match &scr {
        Some(s) => {
            kprintln!(
                "[pc] GOP framebuffer {}x{} (stride {}) at {:#x}, {}",
                s.w, s.h, s.stride, s.base as usize, if s.bgr { "BGR" } else { "RGB" }
            );
        }
        None => kprintln!("[pc] GOP: no drawable framebuffer, so serial console only"),
    }

    // The memory map, read last: nothing may ask the firmware for memory after this
    // and before exit_boot_services, or the key goes stale.
    // SAFETY: MAP_BUF is only touched here, before any other code runs.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(MAP_BUF) };
    let mut map = match u.memory_map(buf) {
        Ok(m) => m,
        Err(e) => {
            kprintln!("[pc] UEFI GetMemoryMap failed (status {:#x}); halted", e);
            cpu::halt_forever();
        }
    };
    let (mut n, mut usable, mut big, mut big_at) = (0usize, 0u64, 0u64, 0u64);
    for d in map.entries() {
        n += 1;
        stackinfo::note_region(d.phys_start, d.pages, d.ty);
        if uefi::usable_after_exit(d.ty) {
            usable += d.pages * 4096;
            // the highest free page below 1 MiB: where a second core's start-up code goes.
            // Conventional memory first; boot-services code/data (types 3, 4: free once the
            // firmware has exited) as the fallback; real firmware often marks low memory so.
            // Below the 640 KiB video hole, never page 0, never the page we are standing on.
            // (QEMU reports all of low memory as one block from address 0: it counts.)
            // The page below it is taken too: core 1 starts on a copy of the kernel's top-level
            // page table there (smp.rs), since its start-up code loads CR3 in 32-bit mode.
            if matches!(d.ty, 3 | 4 | 7) && d.phys_start < 0xA_0000 {
                let end = (d.phys_start + d.pages * 4096).min(0xA_0000);
                let mut page = end - 4096;
                let sp: u64;
                // SAFETY: reads the stack pointer.
                unsafe { core::arch::asm!("mov {}, rsp", out(reg) sp) };
                let sp = sp & !0xFFF;
                while page >= d.phys_start + 4096 && (page == sp || page - 4096 == sp) {
                    page -= 4096;
                }
                if page >= d.phys_start + 4096 && page >= 0x2000 && page != sp && page - 4096 != sp {
                    let slot = if d.ty == 7 { &smp::LOW_PAGE } else { &smp::LOW_PAGE_BS };
                    slot.fetch_max(page, core::sync::atomic::Ordering::Relaxed);
                }
            }
            if d.ty == 7 && d.pages * 4096 > big {
                big = d.pages * 4096;
                big_at = d.phys_start;
            }
        }
    }
    kprintln!(
        "[pc] UEFI memory map: {} entries, {} MiB usable after exit, largest free region {} MiB at {:#x}",
        n, usable >> 20, big >> 20, big_at
    );
    RAM_MIB.store((usable >> 20) as usize, Ordering::Relaxed);

    let mut st_exit = u.exit_boot_services(map.key);
    if st_exit != uefi::SUCCESS {
        // The spec allows one retry with a fresh key (the map changed under us).
        match u.memory_map(buf) {
            Ok(m) => {
                map = m;
                st_exit = u.exit_boot_services(map.key);
            }
            Err(e) => st_exit = e,
        }
    }
    if st_exit != uefi::SUCCESS {
        kprintln!("[pc] ExitBootServices failed (status {:#x}); halted", st_exit);
        cpu::halt_forever();
    }
    kprintln!("[pc] boot services exited; the firmware is gone, the kernel owns the machine");
    // The TSC's frequency, measured against the PIT now that the firmware no longer uses it;
    // the shell's clock (`uptime`, `sleep`, `bench`) counts from here.
    timer::init();
    // The frame pool lives inside the image (see mansa_pc_frame_pool), not in RAM the firmware
    // used: that RAM still holds the page tables CR3 points at until the PC has its own.
    let pool_at = core::ptr::addr_of!(mansa_pc_frame_pool) as usize;
    frame::init(pool_at, POOL_FRAMES);
    kprintln!(
        "[pc] frame pool: {} frames ({} MiB) at {:#x}, {} the kernel heap arena ending {:#x}",
        frame::total_frames(),
        frame::total_frames() * 4 / 1024,
        frame::base(),
        if heap::end() <= frame::base() { "above" } else { "NOT above" },
        heap::end()
    );

    cpu::cli();
    cpu::load_gdt();
    kprintln!("[pc] GDT loaded (kernel code selector {:#x})", cpu::cs());

    cpu::load_idt();
    // SAFETY: a deliberate breakpoint trap; the IDT's vector 3 returns past it.
    unsafe { core::arch::asm!("int3") };
    kprintln!(
        "[pc] IDT loaded; breakpoint exception taken and returned ({})",
        cpu::BREAKPOINTS.load(Ordering::Relaxed)
    );

    // The kernel's own page tables, replacing the firmware's (see mmu.rs). Everything up to
    // the end of the highest memory region in the map is mapped 1:1. Device windows (MMIO,
    // types 11/12) are left out: firmware places 64-bit PCI windows far above RAM (q35: past
    // 512 GiB), and the devices this kernel uses so far sit below 4 GiB, which is always mapped.
    let top = map
        .entries()
        .filter(|d| matches!(d.ty, 1..=10 | 14))
        .map(|d| d.phys_start + d.pages * 4096)
        .max()
        .unwrap_or(0);
    let fw_cr3 = mmu::cr3();
    // the boot trail draws through the kernel's own tables from here: a framebuffer above
    // the memory they map 1:1 is mapped first, or the trail stops (never a fault)
    let early_fb = scr.as_ref().map(|s| (s.base as u64, (s.stride * s.h * 4) as u64));
    early_off(false);
    match mmu::init(top) {
        Some((bytes, tables)) => kprintln!(
            "[pc] own page tables: CR3 {:#x} (firmware's was {:#x}), {} GiB mapped 1:1 with 2 MiB pages in {} table frames",
            mmu::cr3(), fw_cr3, bytes >> 30, tables
        ),
        None => {
            kprintln!("[pc] own page tables: could not be built (frame pool exhausted or RAM too high); halted");
            cpu::halt_forever();
        }
    }
    // F8: the firmware region under the stack is ours to scribble on only now (its page tables are retired).
    stackinfo::paint();
    if let (Some((b, len)), Some(s)) = (early_fb, &scr) {
        if mmu::map_mmio(b, len) {
            early_resume(s);
        }
    }
    if mmu::nx() {
        kprintln!("[pc] no-execute: on (EFER.NXE): ring-3 stack and data pages cannot be run as code");
    } else {
        kprintln!("[pc] no-execute: this CPU does not offer it (CPUID 8000_0001h EDX bit 20), ring-3 pages stay executable");
    }
    // Proof the tables are the kernel's: touch pages nobody mapped. Each touch faults, the
    // #PF handler maps a fresh frame, the write runs again; then read every page back both
    // through its virtual address and through the frame's own 1:1 address.
    const PAGES: u64 = 3;
    let mut alias_ok = true;
    for i in 0..PAGES {
        let va = mmu::DEMAND_BASE + i * 4096 + 8;
        let pattern = 0xAF51_0000_0000_0000 | i;
        // SAFETY: inside the demand window, which the #PF handler backs with a fresh frame.
        unsafe { core::ptr::write_volatile(va as *mut u64, pattern) };
        let pa = mmu::translate(va);
        // SAFETY: `pa` is the frame just mapped, reachable through the 1:1 map too.
        let back = pa.map(|p| unsafe { core::ptr::read_volatile(p as *const u64) });
        // SAFETY: as above, now mapped.
        let again = unsafe { core::ptr::read_volatile(va as *const u64) };
        alias_ok &= back == Some(pattern) && again == pattern && pa.map_or(false, |p| p != va);
    }
    kprintln!(
        "[pc] demand paging: {} page faults served at {:#x}, {} of {} pages read back through both addresses ({})",
        mmu::DEMAND_FAULTS.load(Ordering::Relaxed),
        mmu::DEMAND_BASE,
        if alias_ok { PAGES } else { 0 },
        PAGES,
        if alias_ok { "ok" } else { "MISMATCH" }
    );

    let (x2, base) = cpu::lapic_init(0x2_0000);
    cpu::sti();
    while cpu::TICKS.load(Ordering::Relaxed) < 10 {
        cpu::hlt();
    }
    cpu::cli();
    cpu::lapic_timer_stop();
    if x2 {
        kprintln!("[pc] local APIC (x2APIC) timer: {} interrupts taken", cpu::TICKS.load(Ordering::Relaxed));
    } else {
        kprintln!(
            "[pc] local APIC (xAPIC at {:#x}) timer: {} interrupts taken",
            base,
            cpu::TICKS.load(Ordering::Relaxed)
        );
    }

    // Shared kernel code, the same source files as the ARM kernel.
    let (a, b, c) = (sha256::self_test(), sha512::self_test(), ed25519::self_test());
    let ok = |v: bool| if v { "ok" } else { "FAILED" };
    kprintln!(
        "[pc] shared code: SHA-256 {}, SHA-512 {}, Ed25519 {} (the same files as the ARM kernel)",
        ok(a), ok(b), ok(c)
    );
    let v: Vec<u64> = (1..=1000).collect();
    let sum: u64 = v.iter().sum();
    kprintln!(
        "[pc] heap: a Vec of {} allocated through the shared allocator, sum {} ({}), {} B used",
        v.len(), sum, ok(sum == 500_500), heap::used()
    );
    drop(v);
    // The shared heap self-test grows the REAL arena (its growth checks). The ARM kernel runs
    // it only when asked (`heap test`), so a booted shell starts with the arena as reserved;
    // the PC does the same, and runs it at boot only for the rig's boot check (pc-boot).
    if qemu::fw_cfg_file(b"opt/afrisovera/poweroff") {
        heap::selftest();
    }

    // Ring 3: three tiny programs at the CPU's lowest privilege (see user.rs). One uses the
    // system calls and exits; one tries to read kernel memory; one tries a privileged
    // instruction. The last two must be stopped by the kernel, which carries on.
    let ring3_ok = if user::init() {
        let calls0 = user::SYSCALLS.load(Ordering::Relaxed);
        let rc = user::run(user::Prog::Hello, 0);
        let calls = user::SYSCALLS.load(Ordering::Relaxed) - calls0;
        let hello = rc == 7;
        kprintln!(
            "[pc] ring 3: program 1 made {} system calls and exited with code {} ({})",
            calls, rc, if hello { "ok" } else { "WRONG" }
        );
        let secret = core::ptr::addr_of!(MAP_BUF) as u64; // any kernel address will do
        let rc = user::run(user::Prog::Snoop, secret);
        let err = user::LAST_FAULT_ERROR.load(Ordering::Relaxed);
        let at = user::LAST_FAULT_ADDR.load(Ordering::Relaxed);
        // the fault must be the read of exactly that address, from ring 3, of a present page
        let snoop = rc == user::KILLED | 14 && at == secret && err & 0x5 == 0x5;
        kprintln!(
            "[pc] ring 3: program 2 read kernel memory at {:#x} -> page fault (error {:#x}) at {:#x}, program stopped, kernel continues ({})",
            secret, err, at, if snoop { "ok" } else { "NOT STOPPED" }
        );
        let rc = user::run(user::Prog::Cli, 0);
        let cli = rc == user::KILLED | 13;
        kprintln!(
            "[pc] ring 3: program 3 ran a privileged instruction (cli) -> general protection fault, program stopped ({})",
            if cli { "ok" } else { "NOT STOPPED" }
        );
        hello && snoop && cli
    } else {
        kprintln!("[pc] ring 3: could not map the program's pages");
        false
    };

    // Processes (see proc.rs): two programs in separate address spaces that may talk only
    // through capabilities: one may SEND on endpoint 0, the other may RECV. Each also tries
    // what its capabilities do not allow, and must be refused.
    let ipc_ok = match (
        proc::spawn(proc::code(proc::Prog::Recv), &[(0, proc::RIGHT_RECV)], false),
        proc::spawn(proc::code(proc::Prog::Send), &[(0, proc::RIGHT_SEND)], false),
    ) {
        (Some(_), Some(_)) => {
            let (r0, f0) = (proc::REFUSALS.load(Ordering::Relaxed), proc::FAULTS.load(Ordering::Relaxed));
            let exited = proc::run_all();
            let refused = proc::REFUSALS.load(Ordering::Relaxed) - r0;
            let faults = proc::FAULTS.load(Ordering::Relaxed) - f0;
            let ok = exited == 2 && refused == 3 && faults == 0;
            kprintln!(
                "[pc] IPC: 2 processes in separate address spaces, {} exited, {} requests refused for lack of a capability, {} faults ({})",
                exited, refused, faults, if ok { "ok" } else { "WRONG" }
            );
            ok
        }
        _ => {
            kprintln!("[pc] IPC: could not create the processes");
            false
        }
    };

    // The scheduler: two workers that never give the CPU up; only the timer can switch them.
    let sched_ok = match (
        proc::spawn(proc::code(proc::Prog::Worker), &[], true),
        proc::spawn(proc::code(proc::Prog::Worker), &[], true),
    ) {
        (Some(_), Some(_)) => {
            let s0 = proc::TIMER_SWITCHES.load(Ordering::Relaxed);
            cpu::lapic_init(0x2_0000);
            let exited = proc::run_all();
            cpu::lapic_timer_stop();
            let switches = proc::TIMER_SWITCHES.load(Ordering::Relaxed) - s0;
            let ok = exited == 2 && switches >= 2;
            kprintln!(
                "[pc] scheduler: 2 workers time-sliced by the APIC timer, {} switches between them, {} exited ({})",
                switches, exited, if ok { "ok" } else { "WRONG" }
            );
            ok
        }
        _ => {
            kprintln!("[pc] scheduler: could not create the workers");
            false
        }
    };

    // Endpoints are separate (review R1-10): P0 waits on endpoint 1, P1 on endpoint 2; P2
    // sends 0xB2 on endpoint 2 and P3 0xA1 on endpoint 1, so a SEND that woke any waiting
    // receiver would hand P0 the wrong value. P4's only capability names an endpoint past
    // the last one: it is left empty, so its RECV is refused. Graded by pc-ipc from the
    // values each prints: P0 0xa1, P1 0xb2.
    let ep_ok = {
        let spawned = [
            proc::spawn(proc::code(proc::Prog::EpRecv), &[(1, proc::RIGHT_RECV)], false),
            proc::spawn(proc::code(proc::Prog::EpRecv), &[(2, proc::RIGHT_RECV)], false),
            proc::spawn(proc::code(proc::Prog::EpSendB), &[(2, proc::RIGHT_SEND)], false),
            proc::spawn(proc::code(proc::Prog::EpSendA), &[(1, proc::RIGHT_SEND)], false),
            proc::spawn(proc::code(proc::Prog::EpRecv), &[(proc::N_EP + 5, proc::RIGHT_RECV)], false),
        ];
        if spawned.iter().all(|p| p.is_some()) {
            let (r0, f0) = (proc::REFUSALS.load(Ordering::Relaxed), proc::FAULTS.load(Ordering::Relaxed));
            let exited = proc::run_all();
            let refused = proc::REFUSALS.load(Ordering::Relaxed) - r0;
            let faults = proc::FAULTS.load(Ordering::Relaxed) - f0;
            let ok = exited == 5 && refused == 1 && faults == 0;
            kprintln!(
                "[pc] IPC endpoints: {} endpoints, 2 messages on 2 of them, {} exited, {} refused (a capability past the last endpoint), {} faults ({})",
                proc::N_EP, exited, refused, faults, if ok { "ok" } else { "WRONG" }
            );
            ok
        } else {
            let _ = proc::run_all(); // frees whatever was spawned
            kprintln!("[pc] IPC endpoints: could not create the processes");
            false
        }
    };

    if a && b && c && alias_ok && ring3_ok && ipc_ok && sched_ok && ep_ok {
        kprintln!("[pc] MANSA-PC BOOT OK");
        if let Some(s) = &scr {
            s.text(24, 110, "Boot OK: firmware exited, interrupts and timer live,", 0xC0_FFC0, 2);
            s.text(24, 134, "shared kernel code verified.", 0xC0_FFC0, 2);
        }
    } else {
        kprintln!("[pc] MANSA-PC BOOT FAILED: a shared self-test failed");
    }
    // Under the test rig only (see qemu.rs): end the run. A real PC keeps its screen up.
    if qemu::fw_cfg_file(b"opt/afrisovera/poweroff") {
        kprintln!("[pc] powering off (requested by the test rig through fw_cfg)");
        uefi::shutdown(rt);
    }
    // the shell's console and windows on the real screen (screen.rs)
    if let Some(sc) = &scr {
        screen::attach(sc.base as u64, sc.w, sc.h, sc.stride, sc.bgr);
    }
    start_shell();
}

/// PC track step 2: after the boot proof, the same interactive shell as the ARM kernel,
/// on COM1. The set-up mirrors the ARM boot's last steps before `shell::run` (the device
/// registry, the in-RAM filesystem and its seeded apps, the config, the app hashes).
fn start_shell() -> ! {
    // F8 (docs/design-f8-pc-kernel-stack.md): everything from here on runs on the kernel's own 1 MiB stack with an
    // unmapped guard below it, not on the firmware's 128 KiB (the shell ran 486 KB below that region).
    stackinfo::run_on_kernel_stack(shell_on_kernel_stack)
}

extern "sysv64" fn shell_on_kernel_stack() -> ! {
    asili::register("16550-uart", asili::Class::Console, board::UART_BASE as u64, true);
    asili::register("fw-cfg", asili::Class::Firmware, board::FWCFG_BASE as u64, qemu::fw_cfg_read(0, &mut [0u8; 4]));
    asili::register("local-apic", asili::Class::Interrupt, 0, true);
    asili::register("tsc-timer", asili::Class::Timer, 0, true);
    asili::register("cmos-rtc", asili::Class::Timer, board::RTC_BASE as u64, cmos::present());
    asili::register("rdrand", asili::Class::Entropy, 0, rng::has_rdrand());
    // the shared framebuffer console (the ARM kernel's ramfb console), shown by screen.rs
    framebuffer::console_reset();
    screen::present();
    // USB first: a USB Ethernet adapter can be the network (nic.rs)
    xhci::init();
    // virtio devices (PCI on a PC), then DHCP when there is a network, as the ARM boot does
    virtio::init();
    nvme::init();
    ahci::init();
    // a SATA disk whose firmware is set to IDE (compatibility) mode, or an old IDE disk
    ide::init();
    // an SD card in the PC's card reader (SDHCI)
    sdhci::init();
    ps2::init();
    // Intel HD Audio: the PC's own sound chip (used when there is no virtio-sound device)
    hda::init();
    // the PC's power button: a press saves the files and switches off (ACPI fixed feature)
    acpi::power_button_init();
    // the PC's own drivers in the shared device registry (the Tonga device manager, `devices`)
    if nvme::present() {
        asili::register("nvme", asili::Class::Storage, 0, true);
    }
    if ahci::present() {
        asili::register("ahci-sata", asili::Class::Storage, 0, true);
    }
    if ide::present() {
        asili::register("ide-disk", asili::Class::Storage, 0, true);
    }
    if e1000::present() {
        asili::register("intel-e1000", asili::Class::Net, 0, e1000::link_up());
    }
    if rtl8169::present() {
        asili::register("realtek-nic", asili::Class::Net, 0, rtl8169::link_up());
    }
    if sdhci::present() {
        asili::register("sd-card", asili::Class::Storage, 0, true);
    }
    if xhci::msc_present() {
        asili::register("usb-storage", asili::Class::Storage, 0, true);
    }
    if xhci::net_present() {
        asili::register("usb-ethernet", asili::Class::Net, 0, true);
    }
    if xhci::keyboard_present() {
        asili::register("usb-keyboard", asili::Class::Input, 0, true);
    }
    if xhci::mouse_present() {
        asili::register("usb-mouse", asili::Class::Input, 0, true);
    }
    if xhci::tablet_present() {
        asili::register("usb-tablet", asili::Class::Input, 0, true);
    }
    if ps2::present() {
        asili::register("ps2-keyboard", asili::Class::Input, 0, true);
    }
    if ps2::mouse_present() {
        asili::register("ps2-mouse", asili::Class::Input, 0, true);
    }
    if hda::present() {
        asili::register("intel-hda", asili::Class::Audio, 0, true);
    }
    if xhci::audio_present() {
        asili::register("usb-audio", asili::Class::Audio, 0, true);
    }
    if xhci::serial_present() {
        asili::register("usb-serial", asili::Class::Console, 0, true);
    }
    asili::register("gop-display", asili::Class::Display, 0, screen::on());
    if virtio::net_present() {
        // an Intel NIC's frames reach the driver from the timer interrupt (virtio_pci.rs
        // emu_poll), so DHCP runs with the local-APIC timer on
        let was_on = cpu::interrupts_on();
        cpu::lapic_init(0x2_0000);
        cpu::sti();
        virtio::net_dhcp();
        if !was_on {
            cpu::cli();
        }
    }
    // a second core, when the machine has one (the ARM boot's bring_up_secondary)
    smp::bring_up();
    screen::pat_check(); // D1c: WC only when every running core's PAT has it
    vfs::seed();
    for (name, bytes) in nia::BUNDLED {
        vfs::write(name, bytes()); // the bundled apps, from the one table the manifest also uses
    }
    vfs::write("sample.webapp", web::sample());
    vfs::write("sample.apkmeta", daraja::sample());
    asili::register("ramfs", asili::Class::Storage, 0, true);
    if virtio::blk_present() {
        vfs::load_from_disk();
    }
    nia::restore_bundled(); // a differing or missing bundled app gets its built-in copy back
    config::boot_apply();
    // The manifest: each bundled app's hash from its COMPILED-IN bytes, never from the disk copy (a
    // tampered disk copy must not become "integrity OK"; claude/appget, architect's manifest decision).
    for (name, bytes) in nia::BUNDLED {
        integrity::record(name, bytes());
    }
    kprintln!(
        "[pc] starting the AfriSh shell on COM1 (TSC {} MHz, clock {})",
        timer::hz() / 1_000_000,
        if cmos::present() { "CMOS RTC" } else { "none" }
    );
    // The local APIC timer keeps running under the shell: it is what wakes the console's
    // idle `hlt` (cpu::wfi) to poll COM1 again.
    cpu::lapic_init(0x2_0000);
    // the shell takes the screen: drop the boot trail, paint a clean dark frame over the
    // whole screen (the title and the boot ground that framed the centred window, F9), then
    // draw the console on top of it
    early_off(false);
    screen::clear(0x0A_0E14);
    screen::mark();
    screen::present();
    // typed input arrives by COM1's interrupt, as on the ARM kernel (PL011 RX IRQ)
    uart::enable_rx_irq();
    // the touchpad's bytes by IRQ 12 too: a real i8042 holds one byte at a time (F6)
    ps2::enable_aux_irq();
    cpu::sti();
    shell::run();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("[pc] PANIC: {}", info);
    cpu::halt_forever();
}
