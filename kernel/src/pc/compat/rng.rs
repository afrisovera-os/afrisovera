//! PC stand-in for the ARM `rng.rs`: the SAME generator (the shared SP 800-90A HMAC_DRBG over SHA-256 in
//! `drbg.rs`, with its boot KAT, SP 800-90B health tests, reseeding and fail-closed `fill_secret`), fed by the
//! PC's entropy:
//!
//! - **strong sources** (counted, health-tested): `RDSEED` (CPUID.7.0:EBX bit 18), then `RDRAND`
//!   (CPUID.1:ECX bit 30);
//! - **uncounted context**: the TSC, its measured frequency and the CMOS clock; plus TSC jitter.
#![allow(dead_code)]

use core::ptr::addr_of_mut;

use crate::drbg::{Generator, Platform, RngError, Status};

static mut GEN: Generator = Generator::new();

/// True when CPUID.1:ECX bit 30 (RDRAND) is set.
pub fn has_rdrand() -> bool {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID leaf 1 exists on every x86-64 CPU.
    let r = unsafe { core::arch::x86_64::__cpuid(1) };
    r.ecx & (1 << 30) != 0
}

/// True when CPUID.(EAX=7,ECX=0):EBX bit 18 (RDSEED) is set.
pub fn has_rdseed() -> bool {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID leaf 0 exists on every x86-64 CPU.
    let max = unsafe { core::arch::x86_64::__cpuid(0) }.eax;
    if max < 7 {
        return false;
    }
    #[allow(unused_unsafe)]
    // SAFETY: leaf 7 exists (checked above).
    let r = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
    r.ebx & (1 << 18) != 0
}

/// One RDRAND draw (up to 10 retries, as Intel recommends); None if it never succeeds.
pub fn rdrand64() -> Option<u64> {
    if !has_rdrand() {
        return None;
    }
    for _ in 0..10 {
        let v: u64;
        let ok: u8;
        // SAFETY: RDRAND exists (checked above); it writes only its outputs and the flags.
        unsafe {
            core::arch::asm!("rdrand {v}", "setc {ok}", v = out(reg) v, ok = out(reg_byte) ok,
                             options(nomem, nostack));
        }
        if ok != 0 {
            return Some(v);
        }
    }
    None
}

/// One RDSEED draw (RDSEED may legitimately run dry for a moment: up to 100 retries); None if it never succeeds.
pub fn rdseed64() -> Option<u64> {
    if !has_rdseed() {
        return None;
    }
    for _ in 0..100 {
        let v: u64;
        let ok: u8;
        // SAFETY: RDSEED exists (checked above); it writes only its outputs and the flags.
        unsafe {
            core::arch::asm!("rdseed {v}", "setc {ok}", v = out(reg) v, ok = out(reg_byte) ok,
                             options(nomem, nostack));
        }
        if ok != 0 {
            return Some(v);
        }
        core::hint::spin_loop();
    }
    None
}

fn fill_from(buf: &mut [u8], draw: fn() -> Option<u64>) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match draw() {
            Some(v) => {
                let b = v.to_le_bytes();
                let k = (buf.len() - n).min(8);
                buf[n..n + k].copy_from_slice(&b[..k]);
                n += k;
            }
            None => break,
        }
    }
    n
}

struct Pc;

impl Platform for Pc {
    const STRONG: &'static [&'static str] = &["rdseed", "rdrand"];
    const ARCH: &'static [u8] = b"x86_64";

    fn strong(idx: usize, buf: &mut [u8]) -> usize {
        match idx {
            0 => fill_from(buf, rdseed64),
            1 => fill_from(buf, rdrand64),
            _ => 0,
        }
    }

    fn counter() -> u64 {
        crate::timer::rdtsc()
    }

    fn context(out: &mut [u8; 24]) {
        out[..8].copy_from_slice(&crate::timer::rdtsc().to_le_bytes());
        out[8..16].copy_from_slice(&crate::timer::hz().to_le_bytes());
        out[16..24].copy_from_slice(&crate::cmos::epoch().to_le_bytes());
    }
}

fn gen() -> &'static mut Generator {
    // SAFETY: single accessor (the shell's core), as with the old generator.
    unsafe { &mut *addr_of_mut!(GEN) }
}

pub fn next_u32() -> u32 {
    let mut b = [0u8; 4];
    gen().fill::<Pc>(&mut b);
    u32::from_le_bytes(b)
}

pub fn next_u64() -> u64 {
    let mut b = [0u8; 8];
    gen().fill::<Pc>(&mut b);
    u64::from_le_bytes(b)
}

pub fn fill(buf: &mut [u8]) {
    gen().fill::<Pc>(buf);
}

pub fn fill_secret(buf: &mut [u8]) -> Result<(), RngError> {
    gen().fill_secret::<Pc>(buf)
}

pub fn status() -> Status {
    gen().status::<Pc>()
}

pub fn source_names() -> &'static [&'static str] {
    Pc::STRONG
}
