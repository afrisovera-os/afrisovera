//! Random number generation (ARM).
//!
//! The generator is the shared SP 800-90A **HMAC_DRBG over SHA-256** in `drbg.rs` (boot KAT against the NIST CAVP
//! vectors, SP 800-90B health tests, reseeding, a fail-closed `fill_secret`). This file supplies the ARM entropy:
//!
//! - **strong sources** (counted, health-tested): `RNDR` when the CPU has FEAT_RNG (ID_AA64ISAR0_EL1.RNDR), and a
//!   **virtio-rng** device when the host provides one;
//! - **uncounted context**: the generic-timer counter (`CNTPCT_EL0`) and frequency, and a board time source (PL031
//!   on QEMU virt via `board::resolved().rtc`, the BCM system timer on the Pi); plus timer jitter.
//!
//! On a machine with no strong source (QEMU `-cpu cortex-a72` without virtio-rng), `fill` still works, seeded from
//! the timers as before, but `fill_secret` refuses: secrets need counted hardware entropy.

use core::arch::asm;
use core::ptr::addr_of_mut;

use crate::drbg::{Generator, Platform, RngError, Status};

static mut GEN: Generator = Generator::new();

/// Read the generic-timer physical counter (present on both boards).
fn cntpct() -> u64 {
    let v: u64;
    // SAFETY: CNTPCT_EL0 is readable at EL1 and side-effect-free.
    unsafe { asm!("mrs {}, cntpct_el0", out(reg) v, options(nomem, nostack)) };
    v
}

/// Read the generic-timer frequency.
fn cntfrq() -> u64 {
    let v: u64;
    // SAFETY: CNTFRQ_EL0 is readable at EL1 and side-effect-free.
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) v, options(nomem, nostack)) };
    v
}

/// True when ID_AA64ISAR0_EL1.RNDR (bits 63:60) says FEAT_RNG is implemented.
pub fn has_rndr() -> bool {
    let v: u64;
    // SAFETY: ID_AA64ISAR0_EL1 is readable at EL1 and side-effect-free.
    unsafe { asm!("mrs {}, id_aa64isar0_el1", out(reg) v, options(nomem, nostack)) };
    ((v >> 60) & 0xF) >= 1
}

/// One RNDR draw (s3_3_c2_c4_0). The instruction sets NZCV.Z on failure; a few retries, then None.
fn rndr64() -> Option<u64> {
    if !has_rndr() {
        return None;
    }
    for _ in 0..10 {
        let v: u64;
        let nzcv: u64;
        // SAFETY: FEAT_RNG is present (checked above); RNDR only writes its register and the flags.
        unsafe {
            asm!("mrs {v}, s3_3_c2_c4_0", "mrs {f}, nzcv", v = out(reg) v, f = out(reg) nzcv,
                 options(nomem, nostack));
        }
        if nzcv & (1 << 30) == 0 {
            return Some(v);
        }
    }
    None
}

struct Arm;

impl Platform for Arm {
    const STRONG: &'static [&'static str] = &["rndr", "virtio-rng"];
    const ARCH: &'static [u8] = b"arm64";

    fn strong(idx: usize, buf: &mut [u8]) -> usize {
        match idx {
            0 => {
                let mut n = 0;
                while n < buf.len() {
                    match rndr64() {
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
            1 => crate::virtio::virtio_entropy(buf),
            _ => 0,
        }
    }

    fn counter() -> u64 {
        cntpct()
    }

    fn context(out: &mut [u8; 24]) {
        out[..8].copy_from_slice(&cntpct().to_le_bytes());
        out[8..16].copy_from_slice(&cntfrq().to_le_bytes());
        #[cfg(not(feature = "board-raspi"))]
        // SAFETY: the board's resolved RTC base is a mapped, readable MMIO register.
        let ts = unsafe { core::ptr::read_volatile(crate::board::resolved().rtc as *const u32) } as u64;
        #[cfg(feature = "board-raspi")]
        // SAFETY: the BCM2837 system-timer low word, always mapped on the Pi.
        let ts = unsafe { core::ptr::read_volatile(0x3F00_3004 as *const u32) } as u64;
        out[16..24].copy_from_slice(&ts.to_le_bytes());
    }
}

fn gen() -> &'static mut Generator {
    // SAFETY: single accessor (the boot / shell core), as with the old generator.
    unsafe { &mut *addr_of_mut!(GEN) }
}

/// Next 64-bit random value (non-secret uses).
#[allow(dead_code)]
pub fn next_u64() -> u64 {
    let mut b = [0u8; 8];
    gen().fill::<Arm>(&mut b);
    u64::from_le_bytes(b)
}

/// Next 32-bit random value (non-secret uses).
pub fn next_u32() -> u32 {
    let mut b = [0u8; 4];
    gen().fill::<Arm>(&mut b);
    u32::from_le_bytes(b)
}

/// Fill a byte buffer with random data (non-secret uses; never fails).
#[allow(dead_code)]
pub fn fill(buf: &mut [u8]) {
    gen().fill::<Arm>(buf);
}

/// Fill a buffer with SECRET random data (keys, nonces, ephemerals), or refuse. The only call the mesh may use.
#[allow(dead_code)]
pub fn fill_secret(buf: &mut [u8]) -> Result<(), RngError> {
    gen().fill_secret::<Arm>(buf)
}

/// Generator state for `rng status`.
pub fn status() -> Status {
    gen().status::<Arm>()
}

/// Source names, index-aligned with `Status::present`.
pub fn source_names() -> &'static [&'static str] {
    Arm::STRONG
}
