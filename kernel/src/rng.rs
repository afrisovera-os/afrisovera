//! Random number generation.
//!
//! A **SHA-256 hash-DRBG**: output blocks are `SHA-256(state || counter)`, drawn
//! byte by byte and refilled as they are consumed; the counter advances per
//! block. The state is seeded from real hardware entropy gathered at first use:
//! the ARM generic-timer counter (`CNTPCT_EL0`, present on both boards), its
//! frequency, a board time source (PL031 on QEMU virt / the BCM system timer on
//! the Pi), and — when the host provides a **virtio-rng** device — 16 bytes of
//! true device entropy. All of that is folded into the 32-byte state via SHA-256.
//!
//! This is a CSPRNG *construction* — a hash-DRBG over the (vector-validated)
//! SHA-256 primitive, seeded from hardware entropy — suitable for nonces, IDs,
//! tokens, and jitter. HONEST NOTE: it is NOT a formally-audited / NIST-validated
//! DRBG: there is no continuous reseeding schedule, no prediction-resistance, and
//! no on-line health tests. SHA-256 and Ed25519 are the audited primitives; this
//! generator's construction is standard but its parameters are not certified.

use core::arch::asm;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, Ordering};

static mut DRBG_STATE: [u8; 32] = [0; 32]; // the DRBG key / seed
static mut DRBG_CTR: u64 = 0; // block counter
static mut DRBG_BUF: [u8; 32] = [0; 32]; // current SHA-256 output block
static mut DRBG_POS: usize = 32; // bytes consumed from DRBG_BUF (32 = exhausted)
static SEEDED: AtomicBool = AtomicBool::new(false);

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

/// Gather hardware entropy and derive the initial 32-byte DRBG state via SHA-256.
fn seed() {
    // 24 bytes of always-available timer entropy + up to 16 bytes of true device
    // entropy from a virtio-rng device when one is present.
    let mut buf = [0u8; 40];
    buf[..8].copy_from_slice(&cntpct().to_le_bytes());
    buf[8..16].copy_from_slice(&cntfrq().to_le_bytes());
    #[cfg(not(feature = "board-raspi"))]
    let ts = unsafe { core::ptr::read_volatile(crate::board::resolved().rtc as *const u32) } as u64;
    #[cfg(feature = "board-raspi")]
    let ts = unsafe { core::ptr::read_volatile(0x3F00_3004 as *const u32) } as u64;
    buf[16..24].copy_from_slice(&ts.to_le_bytes());

    let mut hw = [0u8; 16];
    crate::virtio::virtio_entropy(&mut hw);
    buf[24..40].copy_from_slice(&hw);

    let d = crate::sha256::digest(&buf);
    // SAFETY: single-accessor (core-0 shell/boot context).
    unsafe {
        (*addr_of_mut!(DRBG_STATE)).copy_from_slice(&d);
        DRBG_CTR = 0;
        DRBG_POS = 32; // force a fresh block on the first draw
    }
    SEEDED.store(true, Ordering::Relaxed);
}

/// Produce the next DRBG output block: SHA-256(state || counter), then bump the
/// counter and reset the read position.
fn generate_block() {
    // SAFETY: single-accessor; fixed-size arrays.
    unsafe {
        let mut input = [0u8; 40];
        input[..32].copy_from_slice(&*addr_of_mut!(DRBG_STATE));
        input[32..40].copy_from_slice(&DRBG_CTR.to_le_bytes());
        let d = crate::sha256::digest(&input);
        (*addr_of_mut!(DRBG_BUF)).copy_from_slice(&d);
        DRBG_CTR = DRBG_CTR.wrapping_add(1);
        DRBG_POS = 0;
    }
}

/// Next random byte from the DRBG (seeds lazily on first use).
fn next_byte() -> u8 {
    if !SEEDED.load(Ordering::Relaxed) {
        seed();
    }
    // SAFETY: single-accessor; DRBG_POS is kept in [0, 32].
    unsafe {
        if DRBG_POS >= 32 {
            generate_block();
        }
        let b = (*addr_of_mut!(DRBG_BUF))[DRBG_POS];
        DRBG_POS += 1;
        b
    }
}

/// Next 64-bit random value.
#[allow(dead_code)] // public API; next_u32 is the current in-tree caller
pub fn next_u64() -> u64 {
    let mut b = [0u8; 8];
    for x in b.iter_mut() {
        *x = next_byte();
    }
    u64::from_le_bytes(b)
}

/// Next 32-bit random value.
pub fn next_u32() -> u32 {
    let mut b = [0u8; 4];
    for x in b.iter_mut() {
        *x = next_byte();
    }
    u32::from_le_bytes(b)
}

/// Fill a byte buffer with random data.
#[allow(dead_code)] // provided for future callers (key / nonce generation)
pub fn fill(buf: &mut [u8]) {
    for x in buf.iter_mut() {
        *x = next_byte();
    }
}

/// Reseed the DRBG state to a fixed value so a given seed reproduces a sequence
/// (for testing / deterministic streams).
#[allow(dead_code)]
pub fn reseed(s: u64) {
    let d = crate::sha256::digest(&s.to_le_bytes());
    // SAFETY: single-accessor.
    unsafe {
        (*addr_of_mut!(DRBG_STATE)).copy_from_slice(&d);
        DRBG_CTR = 0;
        DRBG_POS = 32;
    }
    SEEDED.store(true, Ordering::Relaxed);
}
