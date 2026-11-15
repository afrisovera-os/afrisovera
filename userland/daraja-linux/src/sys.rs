//! The bridge's calls into Mansa, in one module (COMPAT_STAGE0_PLAN section 16): when v1 moves native calls to
//! SYSCALL (AFR-0001 7.1), only this file changes.
//!
//! v0.5 entry (AFR-0001 4.1, 4.2, 7.6, 7.7): `int 0x80`, the number in rax, arguments in rdi, rsi, rdx, rcx, r8, r9
//! (the v0 order: argument 3 is rcx), the result in rax. The v0.5 and Daraja calls return the v1 errors (7.2): a value
//! from -4095 to -1 is an error, anything else a result. Every register but rax is preserved by the kernel.

use core::arch::asm;

/// A v1 error (AFR-0001 7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(pub i64);
pub const E_PERM: Errno = Errno(-1);
pub const E_INVAL: Errno = Errno(-5);
pub const E_WOULDBLOCK: Errno = Errno(-7);
pub const E_PEERCLOSED: Errno = Errno(-8);
pub const E_TOOBIG: Errno = Errno(-9);

pub type R = Result<u64, Errno>;

/// The bridge's handles, as spawn_pair gives them (AFR-0001 7.7).
pub const FPROC: u64 = 0;
pub const CHAN: u64 = 1;
pub const BUNDLE: u64 = 2;

#[inline(always)]
fn call(n: u64, a: [u64; 6]) -> u64 {
    let r: u64;
    // SAFETY: the Mansa system call gate: the kernel reads only rax and the six argument registers, writes only rax,
    // and checks every pointer argument against this process's own memory before it touches it.
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") n => r,
            in("rdi") a[0], in("rsi") a[1], in("rdx") a[2], in("rcx") a[3], in("r8") a[4], in("r9") a[5],
            options(nostack)
        );
    }
    r
}

fn v1(r: u64) -> R {
    let s = r as i64;
    if (-4095..=-1).contains(&s) {
        Err(Errno(s))
    } else {
        Ok(r)
    }
}

// --- v0 (AFR-0001 5.2) ---------------------------------------------------------------------------------------------
pub fn exit(code: u64) -> ! {
    call(0, [code, 0, 0, 0, 0, 0]);
    loop {}
}
pub fn putc(c: u8) {
    call(11, [c as u64, 0, 0, 0, 0, 0]);
}

// --- v0.5 memory (7.6) ------------------------------------------------------------------------------------------------
pub fn vmo_create(size: u64) -> R {
    v1(call(32, [size, 0, 0, 0, 0, 0]))
}
/// The whole object, read and write: its address.
pub fn vmo_map(h: u64, len: u64) -> R {
    v1(call(33, [h, 0, len, 3, 0, 0]))
}

// --- Daraja (7.7) -----------------------------------------------------------------------------------------------------
pub const PROT_R: u64 = 1;
pub const PROT_RW: u64 = 3;
pub const PROT_RX: u64 = 5;

pub fn channel_write(buf: &[u8]) -> R {
    v1(call(17, [CHAN, buf.as_ptr() as u64, buf.len() as u64, 0, 0, 0]))
}
pub fn channel_read(buf: &mut [u8]) -> R {
    v1(call(18, [CHAN, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0]))
}
pub const SIG_READABLE: u64 = 1;
pub const SIG_PEER_CLOSED: u64 = 4;
/// Wait with no limit for a message or the end; the signals seen.
pub fn wait_one() -> R {
    v1(call(24, [CHAN, SIG_READABLE | SIG_PEER_CLOSED, u64::MAX, 0, 0, 0]))
}
pub fn fproc_map(addr: u64, len: u64, prot: u64) -> R {
    v1(call(49, [FPROC, addr, len, prot, 0, 0]))
}
pub fn fproc_unmap(addr: u64, len: u64) -> R {
    v1(call(50, [FPROC, addr, len, 0, 0, 0]))
}
pub fn fproc_read(faddr: u64, buf: &mut [u8]) -> R {
    v1(call(51, [FPROC, faddr, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0]))
}
pub fn fproc_write(faddr: u64, buf: &[u8]) -> R {
    v1(call(52, [FPROC, faddr, buf.as_ptr() as u64, buf.len() as u64, 0, 0]))
}
pub fn fproc_start(rip: u64, rsp: u64) -> R {
    v1(call(53, [FPROC, rip, rsp, 0, 0, 0]))
}
/// which: 0 FS, 1 GS.
pub fn fproc_set_tls(which: u64, value: u64) -> R {
    v1(call(54, [FPROC, which, value, 0, 0, 0]))
}
pub fn fproc_kill(code: u64) -> R {
    v1(call(55, [FPROC, code, 0, 0, 0, 0]))
}
pub fn fproc_protect(addr: u64, len: u64, prot: u64) -> R {
    v1(call(56, [FPROC, addr, len, prot, 0, 0]))
}
