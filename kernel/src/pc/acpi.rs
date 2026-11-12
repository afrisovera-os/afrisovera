//! ACPI on the PC (5 Nov 2026), just enough for a real machine: the MADT's list of cores
//! (their local-APIC ids) and I/O APICs. Read once at boot from the RSDP that UEFI hands
//! over in its configuration table; the tables live in ACPI memory the kernel never reuses.
//!
//! Why now: waking "all cores but me" with one broadcast STARTUP IPI is fine on QEMU's
//! 2-core machine, but on a real 8-core laptop it would start seven cores on one start-up
//! stack at once. smp.rs now wakes ONE core, by the APIC id listed here.

use core::ptr::read_unaligned;

const MAX_CPUS: usize = 64;
static mut CPUS: [u32; MAX_CPUS] = [0; MAX_CPUS];
static mut NCPUS: usize = 0;
static mut IOAPIC: u64 = 0;
static mut FOUND: bool = false;
/// The ACPI power-management timer's I/O port (3.579545 MHz), from the FADT; 0 = none.
static mut PM_TMR: u16 = 0;

/// The PC's power button as the FADT describes it (7 Nov 2026). A short press sets
/// PWRBTN_STS (bit 8 of PM1_STS) once the OS has set PWRBTN_EN (bit 8 of PM1_EN) and switched
/// the chipset into ACPI mode (SCI_EN, bit 0 of PM1_CNT, set by writing ACPI_ENABLE to
/// SMI_CMD); in the firmware's own (SMM) mode a press may cut the power at once. Polled from
/// the idle loop (cpu::wfi), never in an interrupt: the press latches until cleared.
#[derive(Clone, Copy)]
struct PowerButton {
    smi_cmd: u16,
    acpi_enable: u8,
    evt_a: u16,
    evt_b: u16,
    cnt_a: u16,
    evt_len: u8,
    flags: u32,
    /// "on" once PWRBTN_EN is set and ACPI mode is on; else why not
    state: &'static str,
    acpi_mode_by_us: bool,
    presses: u32,
}
static mut PB: PowerButton = PowerButton {
    smi_cmd: 0,
    acpi_enable: 0,
    evt_a: 0,
    evt_b: 0,
    cnt_a: 0,
    evt_len: 0,
    flags: 0,
    state: "no FADT",
    acpi_mode_by_us: false,
    presses: 0,
};
const PWRBTN: u16 = 1 << 8;
/// FADT flags: bit 4 PWR_BUTTON (the button is a control-method device, not fixed), bit 20
/// HW_REDUCED_ACPI (no PM1 blocks at all).
const F_PWR_BUTTON: u32 = 1 << 4;
const F_HW_REDUCED: u32 = 1 << 20;

unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    core::arch::asm!("in ax, dx", out("ax") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}

/// EFI_ACPI_20_TABLE_GUID 8868e871-e4f1-11d3-bc22-0080c73c8881, as it sits in memory.
const ACPI20: [u8; 16] = [0x71, 0xe8, 0x68, 0x88, 0xf1, 0xe4, 0xd3, 0x11, 0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81];
/// ACPI_TABLE_GUID (1.0) eb9d2d30-2d88-11d3-9a16-0090273fc14d.
const ACPI10: [u8; 16] = [0x30, 0x2d, 0x9d, 0xeb, 0x88, 0x2d, 0xd3, 0x11, 0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d];

unsafe fn rd<T: Copy>(a: u64) -> T {
    read_unaligned(a as *const T)
}

/// An ACPI checksum: the bytes of the structure add up to 0 (mod 256).
unsafe fn sum_ok(a: u64, len: u64) -> bool {
    len > 0 && len < 1 << 20 && (0..len).fold(0u8, |s, i| s.wrapping_add(rd::<u8>(a + i))) == 0
}

/// Find the RSDP in the UEFI configuration table and read the MADT.
pub fn init(n_config: usize, config_table: u64) {
    // SAFETY: boot, single core; the configuration table and the ACPI tables are firmware
    // memory that stays mapped 1:1; every table is checked by signature before use.
    unsafe {
        let mut rsdp = 0u64;
        for i in 0..n_config.min(64) {
            let e = config_table + 24 * i as u64; // { GUID (16), VendorTable (8) }
            let guid: [u8; 16] = rd(e);
            if guid == ACPI20 || (guid == ACPI10 && rsdp == 0) {
                rsdp = rd(e + 16);
            }
        }
        if rsdp == 0 || rd::<[u8; 8]>(rsdp) != *b"RSD PTR " || !sum_ok(rsdp, 20) {
            crate::kprintln!("[pc] ACPI: no valid RSDP from the firmware");
            return;
        }
        let rev: u8 = rd(rsdp + 15);
        let (root, wide) = if rev >= 2 && rd::<u64>(rsdp + 24) != 0 {
            (rd::<u64>(rsdp + 24), true) // XSDT
        } else {
            (rd::<u32>(rsdp + 16) as u64, false) // RSDT
        };
        let len: u32 = rd(root + 4);
        if !sum_ok(root, len as u64) {
            crate::kprintln!("[pc] ACPI: the {} checksum is wrong; not used", if wide { "XSDT" } else { "RSDT" });
            return;
        }
        let step = if wide { 8 } else { 4 };
        let mut madt = 0u64;
        let mut off = 36u64;
        while off + step <= len as u64 {
            let t = if wide { rd::<u64>(root + off) } else { rd::<u32>(root + off) as u64 };
            if t != 0 && rd::<[u8; 4]>(t) == *b"FACP" && sum_ok(t, rd::<u32>(t + 4) as u64) {
                // FADT: PM_TMR_BLK (offset 76, an I/O port), or X_PM_TMR_BLK (offset 208, a
                // Generic Address: space 1 = I/O) when the 32-bit field is empty
                let flen: u32 = rd(t + 4);
                let port32: u32 = rd(t + 76);
                PM_TMR = if port32 != 0 && port32 < 0x10000 {
                    port32 as u16
                } else if flen >= 220 && rd::<u8>(t + 208) == 1 {
                    rd::<u64>(t + 212) as u16
                } else {
                    0
                };
                // the power button's registers (ACPI 6.5 table 5.9): SMI_CMD (48), ACPI_ENABLE
                // (52), PM1a/b_EVT_BLK (56/60), PM1a_CNT_BLK (64), PM1_EVT_LEN (88), Flags (112)
                if flen >= 116 {
                    let io = |o: u64| -> u16 {
                        let v: u32 = rd(t + o);
                        if v < 0x10000 { v as u16 } else { 0 }
                    };
                    PB = PowerButton {
                        smi_cmd: io(48),
                        acpi_enable: rd(t + 52),
                        evt_a: io(56),
                        evt_b: io(60),
                        cnt_a: io(64),
                        evt_len: rd(t + 88),
                        flags: rd(t + 112),
                        ..PB
                    };
                }
            }
            if t != 0 && madt == 0 && rd::<[u8; 4]>(t) == *b"APIC" && sum_ok(t, rd::<u32>(t + 4) as u64) {
                madt = t;
            }
            off += step;
        }
        if madt == 0 {
            crate::kprintln!("[pc] ACPI: no MADT");
            return;
        }
        let mlen: u32 = rd(madt + 4);
        let mut p = madt + 44;
        while p + 2 <= madt + mlen as u64 {
            let (ty, l): (u8, u8) = (rd(p), rd(p + 1));
            if l < 2 {
                break;
            }
            match ty {
                0 if l >= 8 => {
                    let (id, flags): (u8, u32) = (rd(p + 3), rd(p + 4));
                    if flags & 3 != 0 && NCPUS < MAX_CPUS {
                        CPUS[NCPUS] = id as u32;
                        NCPUS += 1;
                    }
                }
                9 if l >= 16 => {
                    let (id, flags): (u32, u32) = (rd(p + 4), rd(p + 8));
                    if flags & 3 != 0 && NCPUS < MAX_CPUS && !CPUS[..NCPUS].contains(&id) {
                        CPUS[NCPUS] = id;
                        NCPUS += 1;
                    }
                }
                1 if l >= 12 && IOAPIC == 0 => IOAPIC = rd::<u32>(p + 4) as u64,
                _ => {}
            }
            p += l as u64;
        }
        FOUND = true;
        crate::kprintln!(
            "[pc] ACPI: {} (rev {}), MADT lists {} core(s), I/O APIC at {:#x}",
            if wide { "XSDT" } else { "RSDT" },
            rev,
            { NCPUS },
            { IOAPIC }
        );
    }
}

/// The local-APIC id of a core other than `me`, to wake: the first one the MADT lists.
/// Without a MADT, id 1 (QEMU's and many machines' second core); a wrong guess just means
/// no core answers and the machine runs on one.
pub fn other_core(me: u32) -> Option<u32> {
    // SAFETY: read-only after init.
    unsafe {
        if !FOUND {
            return if me != 1 { Some(1) } else { Some(0) };
        }
        CPUS[..NCPUS].iter().copied().find(|&id| id != me)
    }
}

/// Cores the MADT lists (0 when there is no MADT).
#[allow(dead_code)]
pub fn cores() -> usize {
    // SAFETY: scalar read.
    unsafe { NCPUS }
}

/// The ACPI PM timer's I/O port (0 = none), for calibrating the TSC without a PIT.
pub fn pm_timer_port() -> u16 {
    // SAFETY: scalar read, written once at boot.
    unsafe { PM_TMR }
}

/// The first I/O APIC's address from the MADT (0 = none found).
pub fn ioapic() -> u64 {
    // SAFETY: scalar read, written once at boot.
    unsafe { IOAPIC }
}

/// Arm the power button (after the drivers, before the shell): ACPI mode on if the firmware
/// left the chipset in its own mode, a stale press cleared, PWRBTN_EN set. One boot line.
pub fn power_button_init() {
    // SAFETY: boot, single core, interrupts off; the ports come from a checksummed FADT and
    // are only touched when the FADT says the fixed-feature button exists.
    unsafe {
        let p = PB;
        let why: Option<&'static str> = if p.evt_a == 0 && p.flags == 0 {
            Some("no FADT")
        } else if p.flags & F_HW_REDUCED != 0 {
            Some("hardware-reduced ACPI: the button is a GPIO device AfriSovera does not drive yet")
        } else if p.evt_a == 0 || p.evt_len < 4 {
            Some("no PM1 event block in the FADT")
        } else if p.flags & F_PWR_BUTTON != 0 {
            Some("a control-method button (needs ACPI's AML, not read yet)")
        } else {
            None
        };
        if let Some(w) = why {
            PB.state = w;
            crate::kprintln!("[pc] power button: not handled ({}); hold it 4 s to switch off", w);
            return;
        }
        if p.cnt_a != 0 && inw(p.cnt_a) & 1 == 0 {
            if p.smi_cmd == 0 || p.acpi_enable == 0 {
                PB.state = "the firmware keeps its own mode and gives no way to switch";
                { let s = PB.state; crate::kprintln!("[pc] power button: not handled ({})", s); }
                return;
            }
            crate::cpu::outb(p.smi_cmd, p.acpi_enable);
            let mut ms = 0;
            while inw(p.cnt_a) & 1 == 0 && ms < 3000 {
                crate::timer::busywait_ms(10);
                ms += 10;
            }
            if inw(p.cnt_a) & 1 == 0 {
                PB.state = "the firmware did not switch to ACPI mode";
                { let s = PB.state; crate::kprintln!("[pc] power button: not handled ({})", s); }
                return;
            }
            PB.acpi_mode_by_us = true;
        }
        for b in [p.evt_a, p.evt_b] {
            if b != 0 {
                crate::cpu::outw(b, PWRBTN); // a press from before: write 1 to clear
                let en = b + (p.evt_len / 2) as u16;
                crate::cpu::outw(en, inw(en) | PWRBTN);
            }
        }
        PB.state = "on";
        crate::kprintln!(
            "[pc] power button: ready; a press saves the files and switches off (PM1a {:#x}, ACPI mode {})",
            p.evt_a,
            if PB.acpi_mode_by_us { "switched on" } else { "already on" }
        );
    }
}

/// Polled by the idle loop: true once per press (the latched status is cleared here).
pub fn power_button_pressed() -> bool {
    // SAFETY: only reads/clears PWRBTN_STS on the ports armed by power_button_init.
    unsafe {
        if PB.state != "on" {
            return false;
        }
        let mut hit = false;
        for b in [PB.evt_a, PB.evt_b] {
            if b != 0 && inw(b) & PWRBTN != 0 {
                crate::cpu::outw(b, PWRBTN);
                hit = true;
            }
        }
        if hit {
            PB.presses += 1;
        }
        hit
    }
}

/// A press: the shell's files are written to the disk in use (as `sync` does), then the
/// firmware switches the PC off (as `poweroff` does).
pub fn power_off_pressed() -> ! {
    crate::kprintln!("[pc] power button pressed: saving AfriSovera's files, then switching off");
    if crate::virtio::blk_present() {
        crate::vfs::sync_to_disk();
        crate::kprintln!("[pc] power button: files saved to the disk");
    } else {
        crate::kprintln!("[pc] power button: no disk in use, nothing to save (files were in memory only)");
    }
    crate::psci::poweroff()
}

/// `acpiinfo`.
pub fn info() {
    // SAFETY: read-only after init.
    unsafe {
        if !FOUND {
            crate::kprintln!("acpi: no MADT found");
            return;
        }
        let (cpus, n, io) = (CPUS, NCPUS, IOAPIC);
        crate::kprintln!("acpi: {} core(s), local-APIC ids {:?}, I/O APIC at {:#x}", n, &cpus[..n], io);
        crate::kprintln!("acpi: PM timer at I/O port {:#x}", { PM_TMR });
        let p = PB;
        crate::kprintln!(
            "acpi: power button {} (PM1a events {:#x}, control {:#x}, SMI_CMD {:#x}, ACPI mode {}, presses {})",
            p.state,
            p.evt_a,
            p.cnt_a,
            p.smi_cmd,
            if p.cnt_a != 0 && inw(p.cnt_a) & 1 != 0 { "on" } else { "off" },
            p.presses
        );
        if let Some(h) = crate::timer::pm_timer_hz() {
            crate::kprintln!("acpi: the PM timer measures the TSC at {} MHz (the clock in use: {} MHz)", h / 1_000_000, crate::timer::hz() / 1_000_000);
        }
    }
}
