//! daraja-linux's program, the bridge itself (step 3, COMPAT_STAGE0_PLAN sections 6, 16 and 16a): the Linux bridge as a native
//! AfriSovera ring-3 program, spawned by spawn_pair with FPROC (0), CHAN (1) and the bundle (2, mapped read-only at
//! the start of its map area). It checks the bundle, loads the program with afri-elf, builds its stack, starts it,
//! and answers its calls with `calls::handle` until it ends. No writable statics: all state is on this stack.
//!
//! Arguments: stage 0 reads them from the bundle file `daraja/argv` (NUL-separated, argv[0] first), which the cloud
//! job writes when it packs the bundle; the program itself is `daraja/argv`'s argv[0], looked up in the bundle. An
//! argument path from the shell's `linux <name> [args]` is its own small 7.7 addition, planned with the test build.

use crate::sys;
use core::arch::global_asm;
use core::ops::Range;
use daraja_linux::{bundle, calls, cpio, limit, stack};

const USER_BASE: u64 = 0x0000_0100_0000_0000;
/// Where the kernel maps the bundle (proc.rs MAP_LO: USER_BASE + 1 GiB).
const BUNDLE_AT: u64 = USER_BASE + 0x4000_0000;
/// Where the program may lie: the first GiB of the user slot, below the map area (the library's FOREIGN_AREA, which
/// fuzz-host's afri_elf target parses under too).
const AREA: Range<u64> = daraja_linux::FOREIGN_AREA;
const _: () = assert!(AREA.start == USER_BASE && AREA.end == BUNDLE_AT);
/// The program's stack: 1 MiB ending just below the map area, with an unmapped page above it.
const STACK_TOP: u64 = USER_BASE + 0x3FFF_F000;
const STACK_LEN: u64 = 1 << 20;
const PAGE: u64 = 4096;

// ------------------------------------------------------------------------------------------------- console --
fn puts(s: &[u8]) {
    for &c in s {
        sys::putc(c);
    }
}
fn hex(v: u64) {
    puts(b"0x");
    let mut started = false;
    for i in (0..16).rev() {
        let d = ((v >> (i * 4)) & 0xF) as u8;
        if d != 0 || started || i == 0 {
            started = true;
            sys::putc(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        }
    }
}
fn dec(v: u64) {
    let mut b = [0u8; 20];
    let mut n = 0;
    let mut x = v;
    loop {
        b[n] = b'0' + (x % 10) as u8;
        n += 1;
        x /= 10;
        if x == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        sys::putc(b[i]);
    }
}
fn fail(why: &[u8]) -> ! {
    puts(b"[daraja] bridge stopped: ");
    puts(why);
    sys::putc(b'\n');
    let _ = sys::fproc_kill(127);
    sys::exit(1)
}

// ----------------------------------------------------------------------------------------------------- host --
/// The kernel as `calls::Host`: the program's memory through fproc_read and fproc_write, its output as console
/// lines `[daraja] out1 <hex bytes>` (exact bytes for the cloud job), its FS base through fproc_set_tls.
struct Kernel;
impl calls::Host for Kernel {
    fn read_mem(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), ()> {
        if buf.is_empty() {
            return Ok(());
        }
        sys::fproc_read(addr, buf).map(|_| ()).map_err(|_| ())
    }
    fn write_mem(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()> {
        if buf.is_empty() {
            return Ok(());
        }
        sys::fproc_write(addr, buf).map(|_| ()).map_err(|_| ())
    }
    fn out(&mut self, fd: u8, bytes: &[u8]) {
        puts(if fd == 1 { b"[daraja] out1 " } else { b"[daraja] out2 " });
        for &b in bytes {
            let d = |x: u8| if x < 10 { b'0' + x } else { b'a' + x - 10 };
            sys::putc(d(b >> 4));
            sys::putc(d(b & 0xF));
        }
        sys::putc(b'\n');
    }
    fn set_fs(&mut self, value: u64) -> Result<(), ()> {
        sys::fproc_set_tls(0, value).map(|_| ()).map_err(|_| ())
    }
}

// ----------------------------------------------------------------------------------------------- the bundle --
fn the_bundle() -> &'static [u8] {
    let mut h = [0u8; bundle::HEADER];
    // SAFETY: the kernel maps at least one page of the bundle at BUNDLE_AT before this program starts (spawn_pair's
    // Bundle grant), read-only; 48 bytes are inside that page.
    unsafe { core::ptr::copy_nonoverlapping(BUNDLE_AT as *const u8, h.as_mut_ptr(), bundle::HEADER) };
    let hdr = match bundle::check(&h) {
        Ok(hdr) => hdr,
        Err(bundle::Refused::BadMagic) => fail(b"bundle refused: not DARAJA02"),
        Err(bundle::Refused::ReservedNotZero) => fail(b"bundle refused: reserved header bytes not 0"),
        Err(bundle::Refused::LengthPastMapping) => fail(b"bundle refused: the archive is longer than its mapping"),
    };
    // SAFETY: bundle::check proved 48 + archive_len fits in the pages the kernel mapped from BUNDLE_AT, and the
    // mapping is read-only and lives as long as this process.
    let ar = unsafe {
        core::slice::from_raw_parts((BUNDLE_AT + bundle::HEADER as u64) as *const u8, hdr.archive_len)
    };
    if cpio::check(ar).is_err() {
        fail(b"bundle refused: the cpio archive is damaged");
    }
    ar
}

// ---------------------------------------------------------------------------------------------- loading --
fn load(file: &[u8]) -> afri_elf::Image {
    let img = match afri_elf::parse(file, AREA) {
        Ok(img) => img,
        // a PT_LOAD segment below the foreign area, usually a program linked at Linux's 0x400000 (ET_EXEC as such is
        // fine: our own builds are ET_EXEC linked high). The low range needs the upper-half kernel first
        // (COMPAT_LOW_TIB_PLAN 6; the architect's wording)
        Err(afri_elf::Error::OutsideArea) => fail(b"this program needs fixed addresses below 1 TiB; not supported yet"),
        // a segment ending above the foreign area (the architect, 21 Nov: "below 1 TiB" would be wrong there)
        Err(afri_elf::Error::AboveArea) => {
            fail(b"this program needs addresses outside the area AfriSovera gives programs; not supported")
        }
        Err(_) => fail(b"the program is not a static x86-64 executable inside the foreign area (afri-elf refused it)"),
    };
    for s in img.segments() {
        let pages = s.pages();
        let len = pages.end - pages.start;
        if sys::fproc_map(pages.start, len, sys::PROT_RW).is_err() {
            fail(b"fproc_map refused a segment");
        }
        // the file bytes; the rest of the segment is already zero (fproc_map gives zeroed frames)
        let bytes = &file[s.offset as usize..(s.offset + s.filesz) as usize];
        let mut done = 0usize;
        while done < bytes.len() {
            let k = (bytes.len() - done).min(64 * 1024);
            if sys::fproc_write(s.vaddr + done as u64, &bytes[done..done + k]).is_err() {
                fail(b"fproc_write refused a segment's bytes");
            }
            done += k;
        }
        // the seal: write dropped from every segment that is not writable; execute only where the file says
        let prot = if s.flags & afri_elf::PF_X != 0 {
            Some(sys::PROT_RX)
        } else if s.flags & afri_elf::PF_W == 0 {
            Some(sys::PROT_R)
        } else {
            None
        };
        if let Some(p) = prot {
            if sys::fproc_protect(pages.start, len, p).is_err() {
                fail(b"fproc_protect refused a segment's seal");
            }
        }
    }
    img
}

// ---------------------------------------------------------------------------------------------- main --
// The entry: the kernel starts a native program with rsp at its stack top (16-byte aligned). `call` pushes the
// return address, so rust_main starts with rsp 8 off a 16-byte boundary, as the System V ABI expects of any function
// (rustapp's `jmp` does not; COMPAT_STAGE0_PLAN 16a).
global_asm!(
    r#"
.section .text.entry
.global _start
_start:
    and rsp, -16
    call rust_main
    ud2
"#
);

#[no_mangle]
pub extern "C" fn rust_main() -> ! {
    let ar = the_bundle();
    // argv from daraja/argv: NUL-separated, argv[0] is the program's path in the bundle
    let Some(cpio::Entry::File(argv_raw)) = cpio::find(ar, b"daraja/argv") else { fail(b"no daraja/argv in the bundle") };
    let mut args: [&[u8]; 16] = [&[]; 16];
    let mut argc = 0usize;
    for a in argv_raw.split(|&c| c == 0).filter(|a| !a.is_empty()) {
        if argc == args.len() {
            fail(b"more than 16 arguments");
        }
        args[argc] = a;
        argc += 1;
    }
    if argc == 0 {
        fail(b"daraja/argv is empty");
    }
    let mut norm = [0u8; 4096];
    let Some(path) = cpio::normalise(args[0], &mut norm) else { fail(b"argv[0] leaves the bundle") };
    let Some(cpio::Entry::File(file)) = cpio::find(ar, path) else { fail(b"argv[0] is not a file in the bundle") };

    let img = load(file);

    // the stack: 1 MiB, RW, never executable; its top page built here and written in
    if sys::fproc_map(STACK_TOP - STACK_LEN, STACK_LEN, sys::PROT_RW).is_err() {
        fail(b"fproc_map refused the stack");
    }
    let mut top = [0u8; PAGE as usize];
    let random: [u8; 16] = {
        let mut h = [0u8; bundle::HEADER];
        // SAFETY: as in the_bundle (the header was checked there).
        unsafe { core::ptr::copy_nonoverlapping(BUNDLE_AT as *const u8, h.as_mut_ptr(), bundle::HEADER) };
        let mut r = [0u8; 16];
        r.copy_from_slice(&h[16..32]);
        r
    };
    let aux = stack::Aux { phdr: img.phdr_vaddr, phent: img.phent, phnum: img.phnum, entry: img.entry };
    let sp = match stack::build(&mut top, STACK_TOP, &args[..argc], &[], &aux, &random) {
        Ok(sp) => sp,
        Err(_) => fail(b"the arguments do not fit in the stack's top page"),
    };
    if sys::fproc_write(STACK_TOP - PAGE, &top).is_err() {
        fail(b"fproc_write refused the stack");
    }
    if sys::fproc_start(img.entry, sp).is_err() {
        fail(b"fproc_start refused");
    }

    // the calls, until the end
    let mut files = calls::Files::new(ar);
    let mut host = Kernel;
    let mut msg = [0u8; 96];
    // `[daraja] unknown call <n>`: the first few per number, then one summary line per number at the end
    let mut unknown = limit::Limit::new();
    loop {
        match sys::wait_one() {
            Ok(s) if s & sys::SIG_READABLE != 0 => {}
            _ => {
                // PEER_CLOSED: the program has ended and its last message was read
                not_shown(&mut unknown);
                sys::exit(0)
            }
        }
        if sys::channel_read(&mut msg).is_err() {
            continue;
        }
        let kind = u16::from_le_bytes([msg[2], msg[3]]);
        let word = |at: usize| u64::from_le_bytes(msg[at..at + 8].try_into().unwrap_or([0; 8]));
        let (seq, nr) = (word(8), word(16));
        match kind {
            1 => {
                let a = [word(24), word(32), word(40), word(48), word(56), word(64)];
                puts(b"[daraja] call ");
                dec(nr);
                sys::putc(b'\n');
                let (rk, value) = match calls::handle(&mut host, &mut files, nr, a) {
                    calls::Action::Ret(v) => {
                        if v == -calls::ENOSYS && unknown.note(nr) == limit::Say::Show {
                            puts(b"[daraja] unknown call ");
                            dec(nr);
                            sys::putc(b'\n');
                        }
                        (1u16, v as u64)
                    }
                    calls::Action::Exit(code) => {
                        not_shown(&mut unknown);
                        puts(b"[daraja] exit ");
                        dec(code);
                        sys::putc(b'\n');
                        (2u16, code)
                    }
                };
                let mut r = [0u8; 24];
                r[0..2].copy_from_slice(&1u16.to_le_bytes());
                r[2..4].copy_from_slice(&rk.to_le_bytes());
                r[8..16].copy_from_slice(&seq.to_le_bytes());
                r[16..24].copy_from_slice(&value.to_le_bytes());
                let _ = sys::channel_write(&r);
            }
            2 => {
                // FAULT: Linux's shell reports 128 + the signal (section 16a)
                let sig = match nr {
                    13 | 14 => 11, // general protection, page fault: SIGSEGV
                    6 => 4,        // invalid opcode: SIGILL
                    0 => 8,        // divide error: SIGFPE
                    3 => 5,        // breakpoint: SIGTRAP
                    _ => 11,
                };
                not_shown(&mut unknown);
                puts(b"[daraja] exit ");
                dec(128 + sig);
                puts(b" (fault, vector ");
                dec(nr);
                puts(b")\n");
            }
            // EXITED: the exit line went out with the EXIT reply; a program killed instead (fproc_kill, the kernel's
            // clean-up) still gets its summary here. take() gives each count once, so nothing prints twice.
            _ => not_shown(&mut unknown),
        }
    }
}

/// The summary lines of the unknown-call limit: one per number whose lines were cut, then one for the rest. Each
/// count is given once, so a later call prints only what came after.
fn not_shown(l: &mut limit::Limit) {
    while let Some((nr, n)) = l.take() {
        match nr {
            Some(nr) => {
                puts(b"[daraja] unknown call ");
                dec(nr);
                puts(b": ");
            }
            None => puts(b"[daraja] unknown calls of other numbers: "),
        }
        dec(n as u64);
        puts(b" more not shown\n");
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail(b"panic")
}

// keep `hex` for the debug lines a later step adds
#[allow(dead_code)]
fn _keep() {
    hex(0);
}
