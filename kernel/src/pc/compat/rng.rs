//! PC stand-in for the ARM `rng.rs`: the SAME construction (a SHA-256 hash-DRBG, output
//! blocks SHA-256(state || counter)) seeded from the PC's entropy instead of the ARM
//! generic timer and virtio-rng: the TSC, its measured frequency, the CMOS clock, and 32
//! bytes from the CPU's RDRAND instruction when CPUID says it has one.
//! HONEST NOTE, as on ARM: a standard construction, not a certified DRBG.
#![allow(dead_code)]

use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, Ordering};

static mut DRBG_STATE: [u8; 32] = [0; 32];
static mut DRBG_CTR: u64 = 0;
static mut DRBG_BUF: [u8; 32] = [0; 32];
static mut DRBG_POS: usize = 32;
static SEEDED: AtomicBool = AtomicBool::new(false);

/// True when CPUID.1:ECX bit 30 (RDRAND) is set.
pub fn has_rdrand() -> bool {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID leaf 1 exists on every x86-64 CPU.
    let r = unsafe { core::arch::x86_64::__cpuid(1) };
    r.ecx & (1 << 30) != 0
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

fn seed() {
    let mut buf = [0u8; 56];
    buf[..8].copy_from_slice(&crate::timer::rdtsc().to_le_bytes());
    buf[8..16].copy_from_slice(&crate::timer::hz().to_le_bytes());
    buf[16..24].copy_from_slice(&crate::cmos::epoch().to_le_bytes());
    for i in 0..4 {
        let v = rdrand64().unwrap_or_else(crate::timer::rdtsc);
        buf[24 + i * 8..32 + i * 8].copy_from_slice(&v.to_le_bytes());
    }
    let d = crate::sha256::digest(&buf);
    // SAFETY: single accessor (the shell's core).
    unsafe {
        (*addr_of_mut!(DRBG_STATE)).copy_from_slice(&d);
        DRBG_CTR = 0;
        DRBG_POS = 32;
    }
    SEEDED.store(true, Ordering::Relaxed);
}

fn next_byte() -> u8 {
    if !SEEDED.load(Ordering::Relaxed) {
        seed();
    }
    // SAFETY: single accessor; DRBG_POS stays in [0, 32].
    unsafe {
        if DRBG_POS >= 32 {
            let mut input = [0u8; 40];
            input[..32].copy_from_slice(&*addr_of_mut!(DRBG_STATE));
            input[32..40].copy_from_slice(&DRBG_CTR.to_le_bytes());
            let d = crate::sha256::digest(&input);
            (*addr_of_mut!(DRBG_BUF)).copy_from_slice(&d);
            DRBG_CTR = DRBG_CTR.wrapping_add(1);
            DRBG_POS = 0;
        }
        let b = (*addr_of_mut!(DRBG_BUF))[DRBG_POS];
        DRBG_POS += 1;
        b
    }
}

pub fn next_u32() -> u32 {
    let mut b = [0u8; 4];
    for x in b.iter_mut() {
        *x = next_byte();
    }
    u32::from_le_bytes(b)
}

pub fn next_u64() -> u64 {
    let mut b = [0u8; 8];
    for x in b.iter_mut() {
        *x = next_byte();
    }
    u64::from_le_bytes(b)
}
