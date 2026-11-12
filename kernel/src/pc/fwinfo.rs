//! `fwinfo` (PC track, 7 Nov 2026): the firmware that started AfriSovera, READ ONLY.
//!
//! For the owner's real-PC try of the USB kit (step 4) and before `bootcopy`/`bootentry`/
//! `bootnext` (step 5): who made the firmware, which UEFI version it speaks, whether its
//! variable services answer (bootentry needs them), and whether Secure Boot is on (then the
//! firmware only starts signed files, and AfriSovera's boot file is not signed). Read once
//! before boot services exit (`keep`); nothing is ever written.
//!
//! SecureBoot (1 = on) and SetupMode (1 = no platform key enrolled) are the UEFI 2.1+ global
//! variables; a firmware without them "does not report" Secure Boot.

use crate::uefi;

const VENDOR_MAX: usize = 48;

/// What a one-byte global variable read gave.
#[derive(Clone, Copy, PartialEq)]
enum Var {
    Value(u8),
    /// NOT_FOUND: the firmware has no such variable
    Absent,
    /// any other status: the variable services did not answer
    Failed,
}

struct Fw {
    kept: bool,
    vendor: [u8; VENDOR_MAX],
    vlen: usize,
    vendor_cut: bool,
    revision: u32,
    uefi: u32,
    secure: Var,
    setup: Var,
}

static mut FW: Fw = Fw {
    kept: false,
    vendor: [0; VENDOR_MAX],
    vlen: 0,
    vendor_cut: false,
    revision: 0,
    uefi: 0,
    secure: Var::Failed,
    setup: Var::Failed,
};

fn byte_var(name: &[u8]) -> Var {
    let mut b = [0u8; 1];
    match uefi::get_global(name, &mut b) {
        Ok(1) => Var::Value(b[0]),
        Ok(_) => Var::Failed,
        Err(uefi::ERR_NOT_FOUND) => Var::Absent,
        Err(_) => Var::Failed,
    }
}

/// Keep the firmware's facts. Before exit_boot_services, after the runtime services are known.
///
/// SAFETY: `st` is the system table the firmware passed in; its vendor string is NUL-ended.
pub unsafe fn keep(st: *const uefi::SystemTable) {
    // SAFETY: only called once, by efi_main, before anything reads FW.
    let fw = unsafe { &mut *core::ptr::addr_of_mut!(FW) };
    // SAFETY: see the function's contract.
    unsafe {
        fw.revision = (*st).firmware_revision;
        fw.uefi = (*st).hdr.revision;
        let mut p = (*st).firmware_vendor;
        while !p.is_null() && *p != 0 {
            if fw.vlen == VENDOR_MAX {
                fw.vendor_cut = true;
                break;
            }
            let c = *p;
            fw.vendor[fw.vlen] = if (0x20..0x7f).contains(&c) { c as u8 } else { b'?' };
            fw.vlen += 1;
            p = p.add(1);
        }
    }
    fw.secure = byte_var(b"SecureBoot");
    fw.setup = byte_var(b"SetupMode");
    fw.kept = true;
    crate::kprintln!("[pc] Secure Boot: {}, kept for fwinfo", secure_word(fw.secure));
}

fn secure_word(v: Var) -> &'static str {
    match v {
        Var::Value(0) => "off",
        Var::Value(1) => "on",
        Var::Value(_) => "an unexpected value",
        Var::Absent => "not reported by this firmware",
        Var::Failed => "unknown (the variable services did not answer)",
    }
}

/// The `fwinfo` command.
pub fn show() {
    // SAFETY: written once before exit_boot_services, only read afterwards.
    let fw = unsafe { &*core::ptr::addr_of!(FW) };
    if !fw.kept {
        crate::kprintln!("fwinfo: the firmware was not read at power-on");
        return;
    }
    crate::kprintln!("fwinfo: the firmware that started AfriSovera, as read at power-on (nothing is changed)");
    crate::kprintln!(
        "  vendor: {}{} (revision {:#x})",
        core::str::from_utf8(&fw.vendor[..fw.vlen]).unwrap_or("?"),
        if fw.vendor_cut { "..." } else { "" },
        fw.revision
    );
    let (major, minor) = (fw.uefi >> 16, fw.uefi & 0xffff);
    if minor % 10 == 0 {
        crate::kprintln!("  UEFI version: {}.{}", major, minor / 10);
    } else {
        crate::kprintln!("  UEFI version: {}.{}.{}", major, minor / 10, minor % 10);
    }
    let answers = fw.secure != Var::Failed || fw.setup != Var::Failed || uefi::boot_vars().read;
    crate::kprintln!(
        "  variable services: {}",
        if answers { "working" } else { "not answering" }
    );
    match fw.secure {
        Var::Value(v) if v > 1 => crate::kprintln!("  Secure Boot: an unexpected value ({})", v),
        v => crate::kprintln!("  Secure Boot: {}", secure_word(v)),
    }
    crate::kprintln!(
        "  Setup Mode: {}",
        match fw.setup {
            Var::Value(0) => "no (a platform key is enrolled)",
            Var::Value(1) => "yes (no platform key enrolled)",
            Var::Value(_) => "an unexpected value",
            Var::Absent => "not reported by this firmware",
            Var::Failed => "unknown",
        }
    );
    const START: &str = "the firmware can start AfriSovera from its own boot entry (bootplan, bootcopy, bootentry, bootnext)";
    if !answers {
        crate::kprintln!("  for the owner: the firmware's variable services do not answer; bootentry and bootnext cannot work on this PC; start AfriSovera from the USB stick");
        return;
    }
    match fw.secure {
        Var::Value(1) => crate::kprintln!("  for the owner: Secure Boot is on; this firmware only starts signed files and AfriSovera's boot file is not signed; switch Secure Boot off in the firmware set-up before bootentry"),
        Var::Value(0) if fw.setup == Var::Value(0) => crate::kprintln!("  for the owner: Secure Boot is off (keys are enrolled, so it can be switched back on later); {}", START),
        Var::Value(0) => crate::kprintln!("  for the owner: Secure Boot is off; {}", START),
        Var::Absent => crate::kprintln!("  for the owner: this firmware has no Secure Boot to switch off; {}", START),
        _ => crate::kprintln!("  for the owner: the firmware gave an unclear Secure Boot answer; look in its set-up screen before bootentry"),
    }
}
