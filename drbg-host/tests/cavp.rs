//! HMAC_DRBG (kernel/src/drbg.rs) against the NIST CAVP files, plus the streaming SHA-256, HMAC, the SP 800-90B
//! health tests and the fail-closed generator. The CAVP JSON comes from scripts/drbg-cavp-fetch.sh (cloud only):
//! DRBG_CAVP_DIR=build/drbg-cavp holds no_reseed.json and pr_false.json, parsed from the NIST .rsp bytes.
use drbg_host::drbg::{hmac_parts, Generator, HmacDrbg, Platform, RngError};
use drbg_host::sha256::{digest, Sha256};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

fn hx(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn load(name: &str) -> Vec<serde_json::Value> {
    let dir = std::env::var("DRBG_CAVP_DIR").expect("DRBG_CAVP_DIR must point at the parsed NIST CAVP JSON");
    let p = std::path::Path::new(&dir).join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {}", p.display(), e));
    serde_json::from_str::<Vec<serde_json::Value>>(&t).unwrap()
}

fn field(e: &serde_json::Value, k: &str) -> Vec<u8> {
    hx(e.get(k).and_then(|v| v.as_str()).unwrap_or(""))
}

fn adds(e: &serde_json::Value) -> Vec<Vec<u8>> {
    e["AdditionalInput"].as_array().unwrap().iter().map(|v| hx(v.as_str().unwrap())).collect()
}

fn run_entry(e: &serde_json::Value, reseed: bool) -> bool {
    let mut d = HmacDrbg::instantiate(&field(e, "EntropyInput"), &field(e, "Nonce"), &field(e, "PersonalizationString"));
    if reseed {
        d.reseed(&field(e, "EntropyInputReseed"), &field(e, "AdditionalInputReseed"));
    }
    let want = field(e, "ReturnedBits");
    let a = adds(e);
    assert_eq!(a.len(), 2, "CAVP HMAC_DRBG entries have two Generate calls");
    let mut out = vec![0u8; want.len()];
    d.generate(&mut out, &a[0]).unwrap();
    d.generate(&mut out, &a[1]).unwrap();
    out == want
}

#[test]
fn cavp_no_reseed_all_entries() {
    let es = load("no_reseed.json");
    assert!(es.len() >= 15, "expected the full SHA-256 PR=False no-reseed set, got {}", es.len());
    let bad: Vec<_> = es.iter().enumerate().filter(|(_, e)| !run_entry(e, false)).map(|(i, _)| i).collect();
    println!("CAVP no_reseed: {} entries, {} fail", es.len(), bad.len());
    assert!(bad.is_empty(), "failing entries {:?}", bad);
}

#[test]
fn cavp_pr_false_all_entries() {
    let es = load("pr_false.json");
    assert!(es.len() >= 15, "expected the full SHA-256 PR=False reseed set, got {}", es.len());
    let bad: Vec<_> = es.iter().enumerate().filter(|(_, e)| !run_entry(e, true)).map(|(i, _)| i).collect();
    println!("CAVP pr_false: {} entries, {} fail", es.len(), bad.len());
    assert!(bad.is_empty(), "failing entries {:?}", bad);
}

#[test]
fn boot_kat_is_count0_and_passes() {
    // The generated constant must be COUNT=0 of the no-reseed file, and the kernel's KAT must pass on it.
    let es = load("no_reseed.json");
    let c0 = es.iter().find(|e| e["count"] == 0).unwrap();
    assert_eq!(field(c0, "EntropyInput"), drbg_host::drbg_kat::KAT_ENTROPY.to_vec());
    assert_eq!(field(c0, "ReturnedBits"), drbg_host::drbg_kat::KAT_RETURNED_BITS.to_vec());
    assert!(drbg_host::drbg::kat());
}

#[test]
fn kat_detects_a_wrong_drbg() {
    // Negative control: one changed entropy byte must change the output, so the KAT can fail.
    let mut e = drbg_host::drbg_kat::KAT_ENTROPY.to_vec();
    e[0] ^= 1;
    let mut d = HmacDrbg::instantiate(&e, &drbg_host::drbg_kat::KAT_NONCE, &drbg_host::drbg_kat::KAT_PERSONALIZATION);
    let mut out = vec![0u8; drbg_host::drbg_kat::KAT_RETURNED_BITS.len()];
    d.generate(&mut out, &drbg_host::drbg_kat::KAT_ADDITIONAL_INPUT_0).unwrap();
    d.generate(&mut out, &drbg_host::drbg_kat::KAT_ADDITIONAL_INPUT_1).unwrap();
    assert_ne!(out, drbg_host::drbg_kat::KAT_RETURNED_BITS.to_vec());
}

#[test]
fn streaming_sha256_matches_digest_and_sha2() {
    use sha2::Digest;
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 31 + 7) as u8).collect();
    for len in [0usize, 1, 55, 56, 63, 64, 65, 119, 120, 127, 128, 129, 1000] {
        let want = sha2::Sha256::digest(&data[..len]);
        assert_eq!(digest(&data[..len]).as_slice(), want.as_slice(), "digest len {}", len);
        for split in [0usize, 1, 7, 63, 64, 65] {
            let s = split.min(len);
            let mut h = Sha256::new();
            h.update(&data[..s]);
            h.update(&data[s..len]);
            assert_eq!(h.finish().as_slice(), want.as_slice(), "stream len {} split {}", len, split);
        }
    }
}

#[test]
fn hmac_matches_rustcrypto() {
    use hmac::Mac;
    for klen in [0usize, 1, 32, 64, 65, 100] {
        let key: Vec<u8> = (0..klen).map(|i| i as u8 ^ 0x5a).collect();
        for mlen in [0usize, 1, 33, 64, 200] {
            let msg: Vec<u8> = (0..mlen).map(|i| (i * 3) as u8).collect();
            let mut m = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(&key).unwrap();
            m.update(&msg);
            let want = m.finalize().into_bytes();
            let (a, b) = msg.split_at(mlen / 2);
            assert_eq!(hmac_parts(&key, &[a, b]).as_slice(), want.as_slice(), "klen {} mlen {}", klen, mlen);
        }
    }
}

#[test]
fn generate_limits() {
    let mut d = HmacDrbg::instantiate(&[1; 32], &[2; 16], b"");
    let mut big = vec![0u8; drbg_host::drbg::MAX_REQUEST + 1];
    assert_eq!(d.generate(&mut big, b""), Err(drbg_host::drbg::DrbgError::RequestTooLong));
}

#[test]
fn health_rct_and_apt_trip() {
    use drbg_host::drbg::Health;
    // A stuck source trips the Repetition Count Test at the cutoff.
    let mut h = Health::new();
    let mut tripped_at = None;
    for i in 0..20 {
        if !h.sample(0x42) {
            tripped_at = Some(i + 1);
            break;
        }
    }
    assert_eq!(tripped_at, Some(drbg_host::drbg::RCT_CUTOFF as usize));
    // A biased source (one value every 4th sample, never adjacent) trips the Adaptive Proportion Test.
    let mut h = Health::new();
    let mut ok = true;
    for i in 0..512u32 {
        let x = if i % 4 == 0 { 0x42 } else { (i % 251) as u8 ^ 0x80 };
        ok &= h.sample(x);
    }
    assert!(!ok && h.failed, "APT must catch 128 copies of one value in a 512 window");
    // A good source passes.
    let mut h = Health::new();
    let mut x: u64 = 0x1234_5678_9abc_def0;
    for _ in 0..100_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        assert!(h.sample((x >> 32) as u8));
    }
}

// --- the generator with mock platforms ---------------------------------------------------------------------

fn xorshift(state: &AtomicU64) -> u64 {
    let mut x = state.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    state.store(x, Ordering::Relaxed);
    x
}

static NONE_CTR: AtomicU64 = AtomicU64::new(1);
struct NoStrong;
impl Platform for NoStrong {
    const STRONG: &'static [&'static str] = &["none"];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, _: &mut [u8]) -> usize {
        0
    }
    fn counter() -> u64 {
        NONE_CTR.fetch_add(3, Ordering::Relaxed)
    }
    fn context(o: &mut [u8; 24]) {
        o[0] = 1;
    }
}

#[test]
fn no_strong_source_fill_works_secret_refuses() {
    let mut g = Generator::new();
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    g.fill::<NoStrong>(&mut a);
    g.fill::<NoStrong>(&mut b);
    assert_ne!(a, b);
    let mut s = [0xffu8; 32];
    assert_eq!(g.fill_secret::<NoStrong>(&mut s), Err(RngError::NoStrongSource));
    assert_eq!(s, [0u8; 32], "a refused secret buffer is zeroed");
}

static GOOD_STATE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
struct Good;
impl Platform for Good {
    const STRONG: &'static [&'static str] = &["good"];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = (xorshift(&GOOD_STATE) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&GOOD_STATE)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn good_source_secret_ok_and_distinct() {
    let mut g = Generator::new();
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    g.fill_secret::<Good>(&mut a).unwrap();
    g.fill_secret::<Good>(&mut b).unwrap();
    assert_ne!(a, b);
    assert_ne!(a, [0u8; 32]);
    let st = g.status::<Good>();
    assert!(st.kat_ok && st.healthy[0] && st.samples[0] >= 1024 && st.secret_ok == 2);
}

struct Stuck;
impl Platform for Stuck {
    const STRONG: &'static [&'static str] = &["stuck"];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        for b in buf.iter_mut() {
            *b = 0xAA;
        }
        buf.len()
    }
    fn counter() -> u64 {
        NONE_CTR.fetch_add(5, Ordering::Relaxed)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn stuck_source_fails_health_and_secret_refuses() {
    let mut g = Generator::new();
    let mut s = [0u8; 16];
    assert_eq!(g.fill_secret::<Stuck>(&mut s), Err(RngError::NoStrongSource));
    let st = g.status::<Stuck>();
    assert!(st.present[0] && !st.healthy[0]);
}

static LATE_CALLS: AtomicUsize = AtomicUsize::new(0);
static LATE_STATE: AtomicU64 = AtomicU64::new(0x0123_4567_89ab_cdef);
struct Late;
impl Platform for Late {
    const STRONG: &'static [&'static str] = &["late"];
    const ARCH: &'static [u8] = b"test";
    fn strong(_: usize, buf: &mut [u8]) -> usize {
        // absent for the first probe (like virtio-rng before the virtio scan), then present
        if LATE_CALLS.fetch_add(1, Ordering::Relaxed) < 1 {
            return 0;
        }
        for b in buf.iter_mut() {
            *b = (xorshift(&LATE_STATE) >> 24) as u8;
        }
        buf.len()
    }
    fn counter() -> u64 {
        xorshift(&LATE_STATE)
    }
    fn context(_: &mut [u8; 24]) {}
}

#[test]
fn a_source_that_appears_later_is_picked_up() {
    let mut g = Generator::new();
    let mut x = [0u8; 8];
    g.fill::<Late>(&mut x); // first use: source absent
    let mut s = [0u8; 32];
    g.fill_secret::<Late>(&mut s).expect("the reseed before a secret re-probes and finds the source");
}

// F1: wipe() zeroes, and a dropped DRBG state (K, V, counter) reads back all zero.
#[test]
fn wipe_and_drop_zero_the_state() {
    let mut b = [0x5au8; 100];
    drbg_host::drbg::wipe(&mut b);
    assert_eq!(b, [0u8; 100]);
    let mut d = std::mem::ManuallyDrop::new(HmacDrbg::instantiate(&[1u8; 32], &[2u8; 16], b"t"));
    let mut out = [0u8; 32];
    d.generate(&mut out, b"").unwrap();
    assert!(!d.is_wiped());
    // SAFETY: dropped exactly once; only plain arrays are read afterwards; ManuallyDrop never drops it again.
    unsafe { std::ptr::drop_in_place(&mut *d) };
    assert!(d.is_wiped());
    // A Sha256 that absorbed a key block no longer holds its chaining value after drop: finishing what is
    // left gives a different digest from the real one (the fields are private, so this is how to see it).
    let mut h = std::mem::ManuallyDrop::new(Sha256::new());
    h.update(&[0x36u8; 64]);
    h.update(b"partial");
    // SAFETY: as above.
    unsafe { std::ptr::drop_in_place(&mut *h) };
    let after = (*h).clone().finish();
    let mut real = Sha256::new();
    real.update(&[0x36u8; 64]);
    real.update(b"partial");
    assert_ne!(after, real.finish());
}
