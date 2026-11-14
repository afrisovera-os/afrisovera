//! F3 step 2a (docs/design-f3-step2a-per-source-entropy.md): per-source entropy claims, cutoffs from H, and a
//! seed that counts bits. The kernel's drbg.rs and drbg_apt.rs are included by path (src/lib.rs).
use drbg_host::drbg::{
    apt_cutoff, rct_cutoff, Generator, Health, Platform, RngError, APT_CUTOFF, H64_DEFAULT, RCT_CUTOFF, SEED_BITS,
};
use drbg_host::drbg_apt::APT_CUTOFF_BY_H64;
use std::sync::atomic::{AtomicU64, Ordering};

fn xorshift(state: &AtomicU64) -> u64 {
    let mut x = state.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    state.store(x, Ordering::Relaxed);
    x
}

#[test]
fn cutoffs_at_four_bits_are_the_old_constants() {
    assert_eq!(H64_DEFAULT, 256);
    assert_eq!(rct_cutoff(H64_DEFAULT), RCT_CUTOFF);
    assert_eq!(apt_cutoff(H64_DEFAULT), APT_CUTOFF);
    assert_eq!((RCT_CUTOFF, APT_CUTOFF), (6, 62));
    let h = Health::new();
    assert_eq!((h.rct_cutoff, h.apt_cutoff), (6, 62));
}

#[test]
fn cutoffs_follow_the_90b_formulas() {
    // RCT: 1 + ceil(20 / H). APT anchors from scripts/drbg-apt-table.py (exact binomial, 80 digits).
    for (h64, rct, apt) in [(512, 4, 13), (256, 6, 62), (128, 11, 177), (64, 21, 311), (16, 81, 468), (8, 161, 497)] {
        assert_eq!(rct_cutoff(h64), rct, "rct at h64={}", h64);
        assert_eq!(apt_cutoff(h64), apt, "apt at h64={}", h64);
    }
    assert_eq!((rct_cutoff(0), apt_cutoff(0)), (0, 0));
    // Condition 1: a claim above 8 bits per byte is invalid, so no cutoff and never counted (no division by zero).
    for bad in [513u32, 4096, u32::MAX] {
        assert_eq!((rct_cutoff(bad), apt_cutoff(bad)), (0, 0), "invalid claim {}", bad);
    }
    // Every valid row: the RCT is the exact integer formula, and the APT row is a real cutoff (the exact binomial
    // check of every row is scripts/drbg-apt-table.py --check, which drbg-cavp runs before these tests).
    for h64 in 1..=512u32 {
        assert_eq!(rct_cutoff(h64), 1 + (1280 + h64 - 1) / h64, "rct at h64={}", h64);
        assert!(apt_cutoff(h64) >= 2 && apt_cutoff(h64) <= 513, "apt at h64={}", h64);
    }
    // The table only falls as H rises, and only its two lowest rows are out of reach of a 512 window.
    assert!(APT_CUTOFF_BY_H64.windows(2).all(|w| w[0] >= w[1]));
    assert_eq!(APT_CUTOFF_BY_H64.iter().filter(|&&c| c > 512).count(), 2);
}

#[test]
fn a_one_eighth_bit_source_still_trips_both_tests() {
    // ADR 0008's cap for jitter is 1/8 bit per delta (H64 = 8): 161 repeats or 497 of 512 must fail it.
    let mut h = Health::for_h64(8);
    for _ in 0..160 {
        assert!(h.sample(7));
    }
    assert!(!h.sample(7), "the 161st identical sample trips the RCT");
    let mut h = Health::for_h64(8);
    let mut tripped = false;
    for i in 0..512u32 {
        // alternate a fresh value in every 50th sample so the RCT never trips, and the APT must
        let x = if i % 50 == 49 { (i / 50) as u8 + 1 } else { 0 };
        if !h.sample(x) {
            tripped = true;
            break;
        }
    }
    assert!(tripped, "the APT trips at 497 of one value in a 512 window");
}

static GOOD: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
struct Four;
impl Platform for Four {
    const STRONG: &'static [&'static str] = &["four"];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = (xorshift(&GOOD) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&GOOD)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn default_claim_draws_64_raw_bytes_for_256_bits() {
    let mut g = Generator::new();
    let mut s = [0u8; 32];
    g.fill_secret::<Four>(&mut s).unwrap();
    let st = g.status::<Four>();
    assert_eq!((st.last_strong, st.last_bits), (64, SEED_BITS));
    assert_eq!((st.h64[0], st.rct_cutoff[0], st.apt_cutoff[0]), (256, 6, 62));
}

static WEAK: AtomicU64 = AtomicU64::new(0x0123_4567_89ab_cdef);
struct Eighth;
impl Platform for Eighth {
    const STRONG: &'static [&'static str] = &["eighth"];
    const H64: &'static [u32] = &[8];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = (xorshift(&WEAK) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&WEAK)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn a_weak_source_is_conditioned_and_counts_256_after_2560_samples() {
    let mut g = Generator::new();
    let before = g.status::<Eighth>().samples[0]; // the start-up test and the first seed
    let mut s = [0u8; 32];
    g.fill_secret::<Eighth>(&mut s).expect("a healthy 1/8-bit source gives a secret through the conditioner");
    let st = g.status::<Eighth>();
    assert_eq!((st.last_strong, st.last_bits), (32, SEED_BITS), "one 32-byte digest, credited 256 bits");
    // 320 bits at 1/8 bit per sample = 2,560 samples per conditioned seed.
    assert!(st.samples[0] - before >= 2560, "drew {} samples", st.samples[0] - before);
    assert_eq!((st.h64[0], st.rct_cutoff[0], st.apt_cutoff[0]), (8, 161, 497));
}

struct Uncounted;
impl Platform for Uncounted {
    const STRONG: &'static [&'static str] = &["uncounted"];
    const H64: &'static [u32] = &[0];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = (xorshift(&WEAK) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&WEAK)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn a_source_claiming_nothing_is_never_counted() {
    let mut g = Generator::new();
    let mut s = [0xffu8; 16];
    assert_eq!(g.fill_secret::<Uncounted>(&mut s), Err(RngError::NoStrongSource));
    assert_eq!(s, [0u8; 16]);
    let st = g.status::<Uncounted>();
    assert!(st.present[0] && st.healthy[0], "present and healthy, just not counted");
    assert_eq!((st.last_strong, st.last_bits), (0, 0));
}

static MIX: AtomicU64 = AtomicU64::new(0x5555_aaaa_3333_cccc);
struct UncountedThenFour;
impl Platform for UncountedThenFour {
    const STRONG: &'static [&'static str] = &["uncounted", "four"];
    const H64: &'static [u32] = &[0, 256];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = (xorshift(&MIX) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&MIX)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn an_uncounted_source_is_skipped_and_the_next_one_counts() {
    let mut g = Generator::new();
    let mut s = [0u8; 16];
    g.fill_secret::<UncountedThenFour>(&mut s).unwrap();
    let st = g.status::<UncountedThenFour>();
    assert_eq!((st.last_strong, st.last_bits), (64, SEED_BITS));
}

struct OverClaim;
impl Platform for OverClaim {
    const STRONG: &'static [&'static str] = &["overclaim"];
    const H64: &'static [u32] = &[513];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = (xorshift(&WEAK) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&WEAK)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn a_claim_above_eight_bits_is_never_counted() {
    // Architect's condition 1: an H64 outside 1 ..= 512 means mixed in, never counted; tested at H = 4.
    let mut g = Generator::new();
    let mut s = [0xffu8; 16];
    assert_eq!(g.fill_secret::<OverClaim>(&mut s), Err(RngError::NoStrongSource));
    let st = g.status::<OverClaim>();
    assert!(st.present[0] && st.healthy[0]);
    assert_eq!((st.last_strong, st.last_bits, st.h64[0]), (0, 0, 0));
    assert_eq!((st.rct_cutoff[0], st.apt_cutoff[0]), (6, 62), "tested at the H = 4 cutoffs");
}
