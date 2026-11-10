//! The PC side of `shell.rs`'s architecture seam (see `kernel/src/shell_arch.rs`).
//!
//! `shell.rs` is the ARM kernel's shell, compiled unchanged into the PC kernel. A few of
//! its commands read ARM-only hardware directly: an MMIO base the PC does not have (the
//! PL031 RTC, PL011 ID registers, GIC, fw_cfg's MMIO window) or an AArch64 system register.
//! `intercept` runs first in `exec` and answers those on the PC: with the PC's REAL
//! equivalent where one is cheap (CMOS clock, CPUID, the 16550's registers, the local
//! APIC, fw_cfg over I/O ports, CR3/IDTR/CR0/CR4/EFER), and otherwise with an honest
//! "not on this PC" line. Every other command runs the shared code as is.

use crate::cpu::{inb, rdmsr};

/// The value an ARM system-register read yields on the PC: there are no ARM system
/// registers, so 0. (`unsafe` only so the `unsafe {}` block around each call site in
/// `shell.rs` stays meaningful on the PC.)
pub unsafe fn sysreg(_name: &str) -> u64 {
    0
}

fn cpuid(leaf: u32, sub: u32) -> (u32, u32, u32, u32) {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID is available on every x86-64 CPU; leaves past the maximum return zeros.
    let r = unsafe { core::arch::x86_64::__cpuid_count(leaf, sub) };
    (r.eax, r.ebx, r.ecx, r.edx)
}

fn cr(n: u8) -> u64 {
    let v: u64;
    // SAFETY: reading a control register in ring 0 has no side effects.
    unsafe {
        match n {
            0 => core::arch::asm!("mov {}, cr0", out(reg) v, options(nomem, nostack)),
            3 => core::arch::asm!("mov {}, cr3", out(reg) v, options(nomem, nostack)),
            _ => core::arch::asm!("mov {}, cr4", out(reg) v, options(nomem, nostack)),
        }
    }
    v
}

fn idtr() -> (u64, u16) {
    let mut d = [0u8; 10];
    // SAFETY: SIDT stores 10 bytes into the buffer.
    unsafe { core::arch::asm!("sidt [{}]", in(reg) d.as_mut_ptr(), options(nostack)) };
    let limit = u16::from_le_bytes([d[0], d[1]]);
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[2..10]);
    (u64::from_le_bytes(b), limit)
}

fn vendor() -> [u8; 12] {
    let (_, b, c, d) = cpuid(0, 0);
    let mut v = [0u8; 12];
    v[..4].copy_from_slice(&b.to_le_bytes());
    v[4..8].copy_from_slice(&d.to_le_bytes());
    v[8..].copy_from_slice(&c.to_le_bytes());
    v
}

fn ascii(b: &[u8]) -> &str {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap_or("?").trim()
}

fn cpuinfo() {
    let v = vendor();
    let (eax, ebx, ecx, edx) = cpuid(1, 0);
    let stepping = eax & 0xF;
    let mut model = (eax >> 4) & 0xF;
    let mut family = (eax >> 8) & 0xF;
    if family == 0xF {
        family += (eax >> 20) & 0xFF;
    }
    if family >= 6 {
        model |= ((eax >> 16) & 0xF) << 4;
    }
    crate::kprintln!("cpuinfo: vendor {} (x86-64 PC)", ascii(&v));
    let (max_ext, _, _, _) = cpuid(0x8000_0000, 0);
    if max_ext >= 0x8000_0004 {
        let mut brand = [0u8; 48];
        for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
            let (a, b, c, d) = cpuid(leaf, 0);
            for (j, r) in [a, b, c, d].iter().enumerate() {
                brand[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&r.to_le_bytes());
            }
        }
        crate::kprintln!("cpuinfo: model name '{}'", ascii(&brand));
    }
    crate::kprintln!("cpuinfo: family 0x{:x} model 0x{:x} stepping 0x{:x}", family, model, stepping);
    crate::kprintln!("cpuinfo: core (APIC id) {}  ring 0", ebx >> 24);
    crate::kprint!("cpuinfo: features");
    for (on, name) in [
        (edx & (1 << 4) != 0, "tsc"),
        (edx & (1 << 9) != 0, "apic"),
        (edx & (1 << 26) != 0, "sse2"),
        (ecx & (1 << 21) != 0, "x2apic"),
        (ecx & (1 << 30) != 0, "rdrand"),
        (ecx & (1 << 25) != 0, "aes"),
    ] {
        if on {
            crate::kprint!(" {}", name);
        }
    }
    crate::kprintln!();
}

/// `rtcinfo`, continued: how the CMOS clock is decoded, and the firmware's own clock
/// (UEFI GetTime) beside it, so a clock that looks wrong on a real PC can be traced to the
/// chip's mode, the time zone the firmware keeps, or this kernel's read.
fn rtc_compare() {
    let sb = crate::cmos::status_b();
    let Some(c) = crate::cmos::civil() else { return };
    crate::kprintln!(
        "rtcinfo: mode {}, {}-hour; raw hour 0x{:02x} minute 0x{:02x} second 0x{:02x}, century 0x{:02x}",
        if sb & 0x04 == 0 { "BCD" } else { "binary" },
        if sb & 0x02 == 0 { 12 } else { 24 },
        c.raw[0], c.raw[1], c.raw[2], c.raw[3]
    );
    crate::kprintln!(
        "rtcinfo: CMOS {:04}-{:02}-{:02} {:02}:{:02}:{:02} (this kernel takes it as UTC)",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    );
    let cmos = crate::cmos::civil_epoch(c.year, c.month, c.day, c.hour, c.minute, c.second) as i64;
    match crate::uefi::get_time() {
        Ok(t) => {
            crate::kprint!(
                "rtcinfo: firmware {:04}-{:02}-{:02} {:02}:{:02}:{:02}, zone ",
                t.year, t.month, t.day, t.hour, t.minute, t.second
            );
            if t.time_zone == 2047 {
                crate::kprint!("unspecified (local time)");
            } else {
                crate::kprint!("{} min", t.time_zone);
            }
            crate::kprintln!(
                ", daylight 0x{:02x} (saving {})",
                t.daylight,
                if t.daylight & 2 != 0 { "in effect" } else { "not in effect" }
            );
            let fw = crate::cmos::civil_epoch(
                t.year as i64, t.month as i64, t.day as i64,
                t.hour as u64, t.minute as u64, t.second as u64,
            ) as i64;
            crate::kprintln!("rtcinfo: CMOS minus firmware: {} s", cmos - fw);
        }
        Err(st) => crate::kprintln!("rtcinfo: firmware clock not readable (status 0x{:x})", st),
    }
}

fn date() {
    let secs = crate::cmos::epoch();
    // Civil UTC from epoch seconds: the same days->civil algorithm as shell.rs's
    // `epoch_to_utc` (private there), so both kernels print identical dates.
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    crate::kprintln!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC  (epoch {})",
        y, mo, d, hh, mm, ss, secs
    );
}

fn not_here(cmd: &str, what: &str) {
    crate::kprintln!("{}: {} -- not present on this PC (x86-64); see 'cpuinfo'", cmd, what);
}

/// Answer `line` on the PC if its ARM form needs ARM-only hardware. Returns true when it
/// was answered here (exec then returns), false to let the shared shell run it.
pub fn intercept(line: &str) -> bool {
    let line = line.trim();
    let (cmd, _rest) = match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    };
    match cmd {
        "date" => date(),
        "lsahci" => crate::ahci::list(),
        "lside" => crate::ide::list(),
        "lsnvme" => crate::nvme::list(),
        "lsparts" => crate::gpt::list(0),
        // the firmware's boot entries as read at power-on (nothing is changed)
        "bootentries" => crate::bootent::list(),
        // the firmware that started AfriSovera: vendor, UEFI version, Secure Boot (nothing is changed)
        "fwinfo" => crate::fwinfo::show(),
        // the PC's network chips, Wi-Fi named with the driver it needs; or one chip by its id (nothing is changed)
        "wifi" => {
            if _rest.is_empty() {
                crate::wifi::show()
            } else {
                crate::wifi::lookup(_rest)
            }
        }
        // where AfriSovera's boot file and boot entry would go next to Windows (reads only)
        "bootplan" => crate::esp::plan(),
        // the boot file copied into the EFI partition Windows starts from, on the owner's code (no boot entry yet); and out again
        "bootcopy" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next(), w.next(), w.next()) {
                (None, _, _, _) => crate::esp::copy(None),
                (Some("yes"), Some(c), None, _) => crate::esp::copy(Some(c)),
                (Some("remove"), None, _, _) => crate::esp::remove(None),
                (Some("remove"), Some("yes"), Some(c), None) => crate::esp::remove(Some(c)),
                _ => crate::kprintln!("bootcopy: type bootcopy (or bootcopy remove) -- it shows what would change and the code; nothing is written without the code"),
            }
        }
        // AfriSovera's own firmware boot entry (Boot#### at the END of BootOrder), on the owner's code; and out again
        "bootentry" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next(), w.next(), w.next()) {
                (None, _, _, _) => crate::bootent::add(None),
                (Some("yes"), Some(c), None, _) => crate::bootent::add(Some(c)),
                (Some("remove"), None, _, _) => crate::bootent::remove(None),
                (Some("remove"), Some("yes"), Some(c), None) => crate::bootent::remove(Some(c)),
                _ => crate::kprintln!("bootentry: type bootentry (or bootentry remove) -- it shows what would change and the code; nothing is written without the code"),
            }
        }
        // AfriSovera started ONCE at the next restart from its own entry (BootNext), on the owner's code
        "bootnext" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next(), w.next()) {
                (None, _, _) => crate::bootent::next(None),
                (Some("yes"), Some(c), None) => crate::bootent::next(Some(c)),
                _ => crate::kprintln!("bootnext: type bootnext -- it shows what would change and the code; nothing is written without the code"),
            }
        }
        "installplan" => match if _rest.is_empty() { Ok(1024) } else { _rest.parse::<u64>() } {
            // where AfriSovera's own partition would go on each internal disk (reads only)
            Ok(m) if (1..=1 << 20).contains(&m) => crate::gpt::list(m),
            _ => crate::kprintln!("installplan: give the size in MiB, 1 to 1048576 (default 1024), e.g. installplan 8192"),
        },
        // the partition tables copied to a USB stick (the internal disks only read), then compared
        "tablebackup" => match _rest {
            "" => crate::gpt::backup(false),
            "new" => crate::gpt::backup(true),
            _ => crate::kprintln!("tablebackup: type tablebackup (or tablebackup new to replace a copy taken before a change)"),
        },
        // the way back: the saved table written onto its disk again, on the owner's code
        "tablerestore" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next(), w.next(), w.next()) {
                (Some(d), None, None, None) => crate::gpt::restore(d, None),
                (Some(d), Some("yes"), Some(c), None) => crate::gpt::restore(d, Some(c)),
                _ => crate::kprintln!("tablerestore: say the disk, e.g. tablerestore nvme (it shows what would change and the code; nothing is written without the code)"),
            }
        }
        "tablecheck" => crate::gpt::check(),
        // the one command that writes an internal disk: AfriSovera's partition, on the owner's code
        "installpart" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next().and_then(|m| m.parse::<u64>().ok()), w.next(), w.next(), w.next()) {
                (Some(d), Some(m), None, None, None) if (1..=1 << 20).contains(&m) => crate::gpt::install(d, m, None),
                (Some(d), Some(m), Some("yes"), Some(c), None) if (1..=1 << 20).contains(&m) => crate::gpt::install(d, m, Some(c)),
                _ => crate::kprintln!("installpart: say the disk and the size in MiB, e.g. installpart nvme 8192 (it shows the plan and the code; nothing is written without the code)"),
            }
        }
        // a whole internal disk given to AfriSovera (a spare PC): boots with no USB stick
        "installdisk" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next(), w.next(), w.next()) {
                (Some(d), None, None, None) => crate::gpt::install_disk(d, None),
                (Some(d), Some("yes"), Some(c), None) => crate::gpt::install_disk(d, Some(c)),
                _ => crate::kprintln!("installdisk: say the disk, e.g. installdisk nvme (it shows what is on it and the code; nothing is written without the code; EVERYTHING on it is erased)"),
            }
        }
        // AfriSovera's partition formatted for the shell's files, on the owner's code
        "formatpart" => {
            let mut w = _rest.split_whitespace();
            match (w.next(), w.next(), w.next(), w.next()) {
                (Some(d), None, None, None) => crate::gpt::format(d, None),
                (Some(d), Some("yes"), Some(c), None) => crate::gpt::format(d, Some(c)),
                _ => crate::kprintln!("formatpart: say the disk, e.g. formatpart nvme (it shows the partition and the code; nothing is written without the code)"),
            }
        }
        // this PC's details sent ONCE to the owner's main PC (tools/report-receiver.py); one-way
        "report" => crate::report::command(_rest),
        "lse1000" => crate::e1000::list(),
        "lsrtl" => crate::rtl8169::list(),
        "lsusb" => crate::xhci::list(),
        "lsps2" => crate::ps2::list(),
        "accel" => crate::ps2::accel_cmd(_rest),
        "usbaddr" => crate::xhci::addr_cmd(_rest),
        // F7 (HP ENVY field report): `help <word>` / `help <page>` show part of the shared list,
        // plain `help` ends with how to; the shared `help` prints it (helpview.rs filters)
        "help" => {
            crate::helpview::start(_rest);
            return false;
        }
        "history" => crate::uart::history_list(),
        "scrollback" => crate::conback::info(),
        "lshda" => crate::hda::list(),
        // the PC's own microphone (HD Audio input): record up to 3 s, then play it back
        "mic" => crate::hda::mic_cmd(_rest),
        "lssd" => crate::sdhci::list(),
        // sound through the PC's own HD Audio chip when there is no virtio-sound device
        // ... or through a USB headset or speaker, preferred when one is plugged in
        "audio" if !crate::virtio_core::sound_present() && crate::xhci::audio_present() => crate::xhci::audio_cmd(_rest),
        "audio" if !crate::virtio_core::sound_present() && crate::hda::present() => crate::hda::audio(_rest),
        "screeninfo" => crate::screen::info(),
        // F9b (HP ENVY field report): how the 640x480 console fills a bigger screen
        "screen" => crate::screen::command(_rest),
        "acpiinfo" => crate::acpi::info(),
        "claimdisk" => {
            // a blank internal disk is only ever used when the owner says so
            let ok = match _rest {
                "nvme" => crate::nvme::claim(),
                "sata" => crate::ahci::claim(),
                "ide" => crate::ide::claim(),
                "sd" | "emmc" => crate::sdhci::claim(),
                _ => {
                    crate::kprintln!("claimdisk: say which -- claimdisk nvme, claimdisk sata, claimdisk ide, claimdisk sd or claimdisk emmc (only a BLANK disk can be claimed)");
                    return true;
                }
            };
            if ok {
                crate::kprintln!("claimdisk: the blank {} disk now holds AfriSovera's files", _rest);
                crate::vfs::sync_to_disk();
            } else {
                crate::kprintln!("claimdisk: no blank {} disk to claim (disks holding data are never touched)", _rest);
            }
        }
        "wmlist" => {
            let mut n = 0;
            for id in 0..4 {
                if let Some((x, y, w, h)) = crate::tonga::win_pos(id) {
                    crate::kprintln!("wm: window {} at {},{} size {}x{}", id, x, y, w, h);
                    n += 1;
                }
            }
            let z = crate::tonga::win_order();
            crate::kprintln!("wm: {} window(s){}, front to back {:?}", n, if crate::screen::wm_view() { ", on screen" } else { "" }, [z[3], z[2], z[1], z[0]]);
        }
        "gui" => {
            crate::screen::set_desktop(true); // its dock answers mouse clicks
            return false; // the shared `gui` draws it
        }
        // F8 (HP ENVY field report): `desk` alone opens the desktop, as `gui` does; its state
        // moved to `desk status`. PC only, so the shared shell.rs (and the ARM image) is unchanged.
        "desk" if _rest.is_empty() => {
            crate::screen::set_desktop(true); // its dock answers mouse clicks
            crate::framebuffer::set_console(false);
            crate::desk::show();
            crate::kprintln!("[desk] desktop opened; type 'desk status' for its state, 'clear' to return to console");
        }
        "desk" if _rest == "status" => crate::desk::command("list"),
        "desk" => {
            // `desk open files` etc. also bring the desktop up (shared code); let its dock take clicks too
            if !crate::desk::shown() && _rest != "list" {
                crate::screen::set_desktop(true);
            }
            return false;
        }
        "clear" => {
            crate::screen::set_wm_view(false);
            crate::screen::set_desktop(false);
            return false; // the shared `clear` still runs
        }
        "cpuinfo" => cpuinfo(),
        "rtcinfo" => {
            crate::kprintln!("rtcinfo: CMOS RTC (MC146818) @ I/O port 0x70");
            crate::kprintln!("rtcinfo: present {}  status-B 0x{:02x}", crate::cmos::present(), crate::cmos::status_b());
            crate::kprintln!("rtcinfo: epoch seconds = {}", crate::cmos::epoch());
            rtc_compare();
        }
        "uartinfo" | "uartflags" => {
            // SAFETY: reads of COM1's line-status / interrupt-id / line-control registers.
            let (lsr, iir, lcr) = unsafe { (inb(0x3FD), inb(0x3FA), inb(0x3FB)) };
            crate::kprintln!("{}: 16550 @ I/O port 0x3f8 (COM1), polled", cmd);
            crate::kprintln!(
                "{}: LSR 0x{:02x} (data-ready {} THR-empty {} overrun {})  IIR 0x{:02x} (FIFO {})  LCR 0x{:02x}",
                cmd, lsr, lsr & 1, (lsr >> 5) & 1, (lsr >> 1) & 1, iir,
                if iir & 0xC0 == 0xC0 { "on" } else { "off" }, lcr
            );
        }
        "gicinfo" | "giccinfo" | "gicid" => {
            // SAFETY: IA32_APIC_BASE is architectural on every x86-64 CPU.
            let base = unsafe { rdmsr(0x1B) };
            crate::kprintln!("{}: no GIC on a PC -- interrupts go through the local APIC", cmd);
            crate::kprintln!(
                "{}: local APIC base 0x{:x}, {} mode, globally {}",
                cmd,
                base & 0xF_FFFF_F000,
                if base & (1 << 10) != 0 { "x2APIC" } else { "xAPIC" },
                if base & (1 << 11) != 0 { "enabled" } else { "disabled" }
            );
        }
        "fwcfg" => {
            let mut sig = [0u8; 4];
            if !crate::qemu::fw_cfg_read(0x0000, &mut sig) {
                crate::kprintln!("fwcfg: no fw_cfg device on this PC (not running under QEMU)");
                return true;
            }
            let mut id = [0u8; 4];
            crate::qemu::fw_cfg_read(0x0001, &mut id);
            let mut nb = [0u8; 4];
            crate::qemu::fw_cfg_read(0x0019, &mut nb);
            crate::kprintln!("fwcfg: signature = {}", core::str::from_utf8(&sig).unwrap_or("?"));
            crate::kprintln!("fwcfg: id = 0x{:08x}", u32::from_le_bytes(id));
            crate::kprintln!("fwcfg: file count = {}", u32::from_be_bytes(nb));
        }
        "smpinfo" => {
            crate::kprintln!("smpinfo: cores online = {}", crate::cores_online());
            crate::kprintln!("smpinfo: the PC kernel runs on the boot processor only (application processors not started yet)");
        }
        "ttbr" => {
            crate::kprintln!("ttbr: no TTBR0/TTBR1 on x86-64; the page-table root is CR3 = 0x{:x}", cr(3));
        }
        "vbar" => {
            let (base, limit) = idtr();
            crate::kprintln!("vbar: no VBAR_EL1 on x86-64; the vector table is the IDT at 0x{:x} (limit 0x{:x})", base, limit);
        }
        "mmuinfo" => {
            let c0 = cr(0);
            crate::kprintln!(
                "mmuinfo: x86-64 CR0 = 0x{:x} (PG paging {}, WP write-protect {}, PE protected mode {})",
                c0, (c0 >> 31) & 1, (c0 >> 16) & 1, c0 & 1
            );
        }
        "tcrinfo" | "cpacr" => {
            // SAFETY: IA32_EFER is architectural on x86-64.
            let efer = unsafe { rdmsr(0xC000_0080) };
            let c4 = cr(4);
            crate::kprintln!(
                "{}: no AArch64 {} on x86-64; CR4 = 0x{:x} (PAE {}, OSFXSR {}), EFER = 0x{:x} (LME {}, NXE {})",
                cmd, if cmd == "cpacr" { "CPACR_EL1" } else { "TCR_EL1" },
                c4, (c4 >> 5) & 1, (c4 >> 9) & 1, efer, (efer >> 8) & 1, (efer >> 11) & 1
            );
        }
        "mairinfo" => {
            // SAFETY: IA32_PAT is architectural on x86-64.
            let pat = unsafe { rdmsr(0x277) };
            crate::kprintln!("mairinfo: no MAIR_EL1 on x86-64; the memory-type table is IA32_PAT = 0x{:016x}", pat);
        }
        "dczinfo" => not_here(cmd, "DCZID_EL0 (AArch64 cache-zero register)"),
        "cacheinfo" | "clidr" | "ccsidr" => not_here(cmd, "AArch64 cache ID registers (CTR/CLIDR/CCSIDR)"),
        "dfr" | "pfr" | "mmfr" | "mmfr1" | "isar" | "isar1" => {
            not_here(cmd, "AArch64 ID_AA64* feature register")
        }
        "pmcr" => not_here(cmd, "PMCR_EL0 (AArch64 performance monitors)"),
        _ => return false,
    }
    true
}
