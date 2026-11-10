//! The kernel's random generator core, shared by the ARM (`rng.rs`) and PC (`pc/compat/rng.rs`) kernels.
//!
//! **Construction:** NIST SP 800-90A HMAC_DRBG over SHA-256, built on the kernel's own `sha256.rs` (no crate).
//! **Entropy:** each architecture supplies "strong" hardware sources (ARM: RNDR, virtio-rng; PC: RDSEED, RDRAND)
//! plus a timer counter used for jitter. Jitter and clock readings are mixed in but never COUNTED as entropy.
//! **Health tests:** SP 800-90B section 4.4 Repetition Count Test and Adaptive Proportion Test run on every byte a
//! strong source delivers, continuously, after a 1024-sample start-up test per source (90B section 4.3). A source
//! that fails is dropped for the rest of the boot (sticky).
//! **Boot KAT:** before first use, the DRBG is run against COUNT=0 of the NIST CAVP HMAC_DRBG SHA-256 no-reseed
//! file (`drbg_kat.rs`, generated from the NIST bytes by `scripts/drbg-cavp-parse.py`, never typed by hand).
//! **Fail closed:** `fill_secret` (the only call the mesh may use for keys, nonces and ephemerals) refuses unless
//! the KAT passed and a healthy strong source delivered fresh entropy for THIS request (prediction resistance:
//! every secret request reseeds first). `fill` (ids, TCP ISS, uuids, `rand`) never fails; it keeps producing
//! output from the DRBG even when no strong source exists, as the old generator did.
//!
//! Assumed min-entropy of a strong source byte: H = 4 bits (conservative for RNDR / RDSEED / a host urandom
//! behind virtio-rng). That gives RCT cutoff C = 1 + ceil(20 / H) = 6 and, for a 512-sample window, APT cutoff
//! C = 1 + CRITBINOM(512, 2^-4, 1 - 2^-20) = 62 (alpha = 2^-20). A reseed asks for 64 strong bytes (256 bits).
//!
//! Single accessor, as before: the generator lives in a `static mut` used from the boot / shell core only.
#![allow(dead_code)]

use crate::sha256::Sha256;

/// SP 800-90A Table 2, HMAC_DRBG: at most 2^19 bits (64 KiB) per Generate.
pub const MAX_REQUEST: usize = 65536;
/// Generates allowed between reseeds (SP 800-90A allows up to 2^48; we reseed far sooner).
pub const RESEED_INTERVAL: u64 = 1 << 16;
/// Strong bytes wanted per (re)seed: 64 bytes at H = 4 bits/byte = 256 bits.
pub const SEED_STRONG_BYTES: usize = 64;
/// SP 800-90B cutoffs for H = 4 bits/sample, alpha = 2^-20 (see the module comment).
pub const RCT_CUTOFF: u32 = 6;
pub const APT_WINDOW: u32 = 512;
pub const APT_CUTOFF: u32 = 62;
/// Start-up test length per source (SP 800-90B section 4.3: at least 1024 samples).
pub const STARTUP_SAMPLES: usize = 1024;
/// At most this many strong sources per architecture.
pub const MAX_SOURCES: usize = 3;

// ---------------------------------------------------------------------------------------------------------------
// Wiping (F1, review S2): every buffer that held entropy, a seed, a key or DRBG state is overwritten with zeros
// before it goes out of scope. Volatile writes plus a compiler fence, so the optimiser cannot drop the stores as
// dead. (Copies the compiler makes on its own, in registers or spill slots, are out of reach of any wipe.)

/// Overwrite `b` with zeros in a way the optimiser may not remove.
pub fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: a valid &mut u8.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

// ---------------------------------------------------------------------------------------------------------------
// HMAC-SHA-256 over byte parts (no concatenation buffer, no length limit)

/// HMAC-SHA-256(key, parts[0] || parts[1] || ...), for a key of at most 64 bytes.
pub fn hmac_parts(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k0 = [0u8; 64];
    if key.len() > 64 {
        k0[..32].copy_from_slice(&crate::sha256::digest(key));
    } else {
        k0[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k0[i];
        opad[i] ^= k0[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    for p in parts {
        inner.update(p);
    }
    let mut ih = inner.finish();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&ih);
    let mac = outer.finish();
    // The padded keys and the inner hash are key material (F1); the Sha256 states wipe themselves on drop.
    wipe(&mut k0);
    wipe(&mut ipad);
    wipe(&mut opad);
    wipe(&mut ih);
    mac
}

// ---------------------------------------------------------------------------------------------------------------
// SP 800-90A HMAC_DRBG (section 10.1.2), SHA-256, no prediction-resistance flag inside (the caller reseeds).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrbgError {
    /// The reseed counter passed RESEED_INTERVAL: the caller must reseed first.
    ReseedRequired,
    /// More than MAX_REQUEST bytes asked for in one Generate.
    RequestTooLong,
}

pub struct HmacDrbg {
    k: [u8; 32],
    v: [u8; 32],
    reseed_counter: u64,
}

impl HmacDrbg {
    pub const fn empty() -> Self {
        HmacDrbg { k: [0; 32], v: [0; 32], reseed_counter: 0 }
    }

    /// HMAC_DRBG_Update (10.1.2.2) with provided_data = the concatenation of `pd`.
    fn update(&mut self, pd: &[&[u8]]) {
        let empty = pd.iter().all(|p| p.is_empty());
        self.k = hmac_parts(&self.k, &[&self.v, &[0x00u8], pd.first().copied().unwrap_or(&[]), rest(pd, 1), rest(pd, 2)]);
        self.v = hmac_parts(&self.k, &[&self.v]);
        if empty {
            return;
        }
        self.k = hmac_parts(&self.k, &[&self.v, &[0x01u8], pd.first().copied().unwrap_or(&[]), rest(pd, 1), rest(pd, 2)]);
        self.v = hmac_parts(&self.k, &[&self.v]);
    }

    /// HMAC_DRBG_Instantiate (10.1.2.3): seed_material = entropy || nonce || personalization.
    pub fn instantiate(entropy: &[u8], nonce: &[u8], pers: &[u8]) -> Self {
        let mut d = HmacDrbg { k: [0x00; 32], v: [0x01; 32], reseed_counter: 1 };
        d.update(&[entropy, nonce, pers]);
        d
    }

    /// HMAC_DRBG_Reseed (10.1.2.4): seed_material = entropy || additional_input.
    pub fn reseed(&mut self, entropy: &[u8], additional: &[u8]) {
        self.update(&[entropy, additional]);
        self.reseed_counter = 1;
    }

    /// HMAC_DRBG_Generate (10.1.2.5).
    pub fn generate(&mut self, out: &mut [u8], additional: &[u8]) -> Result<(), DrbgError> {
        if out.len() > MAX_REQUEST {
            return Err(DrbgError::RequestTooLong);
        }
        if self.reseed_counter > RESEED_INTERVAL {
            return Err(DrbgError::ReseedRequired);
        }
        if !additional.is_empty() {
            self.update(&[additional]);
        }
        let mut pos = 0;
        while pos < out.len() {
            self.v = hmac_parts(&self.k, &[&self.v]);
            let n = (out.len() - pos).min(32);
            out[pos..pos + n].copy_from_slice(&self.v[..n]);
            pos += n;
        }
        self.update(&[additional]);
        self.reseed_counter += 1;
        Ok(())
    }

    /// Wipe the working state. Also runs on drop, so a replaced or KAT instance never leaves K and V behind.
    pub fn wipe(&mut self) {
        wipe(&mut self.k);
        wipe(&mut self.v);
        self.reseed_counter = 0;
    }
}

impl HmacDrbg {
    /// True when K and V are all zero (what `wipe` leaves); for the in-kernel selftest of F1.
    pub fn is_wiped(&self) -> bool {
        self.k == [0u8; 32] && self.v == [0u8; 32] && self.reseed_counter == 0
    }
}

impl Drop for HmacDrbg {
    fn drop(&mut self) {
        self.wipe();
    }
}

fn rest<'a>(pd: &[&'a [u8]], i: usize) -> &'a [u8] {
    if i < pd.len() {
        pd[i]
    } else {
        &[]
    }
}

/// The boot known-answer test: NIST CAVP HMAC_DRBG [SHA-256] [PredictionResistance = False] no-reseed COUNT=0.
/// CAVP procedure: instantiate, generate (output discarded), generate, compare the second output.
pub fn kat() -> bool {
    use crate::drbg_kat::*;
    let mut d = HmacDrbg::instantiate(&KAT_ENTROPY, &KAT_NONCE, &KAT_PERSONALIZATION);
    let mut out = [0u8; KAT_RETURNED_BITS.len()];
    let ok = d.generate(&mut out, &KAT_ADDITIONAL_INPUT_0).is_ok()
        && d.generate(&mut out, &KAT_ADDITIONAL_INPUT_1).is_ok()
        && out == KAT_RETURNED_BITS;
    d.wipe();
    ok
}

// ---------------------------------------------------------------------------------------------------------------
// SP 800-90B health tests (section 4.4), one pair per strong source, on byte samples.

#[derive(Clone, Copy)]
pub struct Health {
    rct_last: u8,
    rct_count: u32,
    apt_first: u8,
    apt_count: u32,
    apt_seen: u32,
    started: bool,
    pub failed: bool,
    pub samples: u64,
}

impl Health {
    pub const fn new() -> Self {
        Health { rct_last: 0, rct_count: 0, apt_first: 0, apt_count: 0, apt_seen: 0, started: false, failed: false, samples: 0 }
    }

    /// Feed one sample; returns false (and latches `failed`) when either test trips.
    pub fn sample(&mut self, x: u8) -> bool {
        self.samples += 1;
        // Repetition Count Test (4.4.1)
        if self.started && x == self.rct_last {
            self.rct_count += 1;
            if self.rct_count >= RCT_CUTOFF {
                self.failed = true;
            }
        } else {
            self.rct_last = x;
            self.rct_count = 1;
        }
        // Adaptive Proportion Test (4.4.2)
        if !self.started || self.apt_seen >= APT_WINDOW {
            self.apt_first = x;
            self.apt_count = 1;
            self.apt_seen = 1;
        } else {
            self.apt_seen += 1;
            if x == self.apt_first {
                self.apt_count += 1;
                if self.apt_count >= APT_CUTOFF {
                    self.failed = true;
                }
            }
        }
        self.started = true;
        !self.failed
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The generator: KAT + start-up tests + seeding + reseeding + the two public fills.

/// What each architecture provides.
pub trait Platform {
    /// Names of the strong sources, in index order (at most MAX_SOURCES).
    const STRONG: &'static [&'static str];
    /// Fill `buf` (at most 64 bytes) from strong source `idx`; return the bytes written. 0 = absent or failed.
    fn strong(idx: usize, buf: &mut [u8]) -> usize;
    /// A free-running high-resolution counter (jitter; never counted as entropy).
    fn counter() -> u64;
    /// Uncounted context mixed into every seed: clocks, frequencies.
    fn context(out: &mut [u8; 24]);
    /// A short architecture tag for the personalization string.
    const ARCH: &'static [u8];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RngError {
    /// The boot known-answer test failed: the DRBG code itself is wrong.
    KatFailed,
    /// No strong hardware source is present and healthy, so no fresh entropy could be counted.
    NoStrongSource,
    /// A request larger than MAX_REQUEST.
    TooLong,
}

pub struct Generator {
    drbg: HmacDrbg,
    seeded: bool,
    kat_ran: bool,
    kat_ok: bool,
    present: [bool; MAX_SOURCES],
    health: [Health; MAX_SOURCES],
    /// Strong bytes counted at the most recent (re)seed.
    pub last_strong: usize,
    pub reseeds: u64,
    pub secret_ok: u64,
    pub secret_refused: u64,
}

pub struct Status {
    pub kat_ok: bool,
    pub seeded: bool,
    pub last_strong: usize,
    pub reseeds: u64,
    pub secret_ok: u64,
    pub secret_refused: u64,
    pub present: [bool; MAX_SOURCES],
    pub healthy: [bool; MAX_SOURCES],
    pub samples: [u64; MAX_SOURCES],
}

impl Generator {
    pub const fn new() -> Self {
        Generator {
            drbg: HmacDrbg::empty(),
            seeded: false,
            kat_ran: false,
            kat_ok: false,
            present: [false; MAX_SOURCES],
            health: [Health::new(); MAX_SOURCES],
            last_strong: 0,
            reseeds: 0,
            secret_ok: 0,
            secret_refused: 0,
        }
    }

    /// Draw up to `want` health-tested bytes from every present, healthy strong source into `dst`; returns the
    /// number of bytes written. A source that trips a health test is dropped for the rest of the boot.
    fn draw_strong<P: Platform>(&mut self, dst: &mut [u8], want: usize) -> usize {
        let mut n = 0;
        let mut chunk = [0u8; 64];
        for i in 0..P::STRONG.len().min(MAX_SOURCES) {
            if !self.present[i] || self.health[i].failed {
                continue;
            }
            let mut got_here = 0;
            while got_here < want && n < dst.len() {
                let ask = (want - got_here).min(64).min(dst.len() - n);
                let g = P::strong(i, &mut chunk[..ask]).min(ask);
                if g == 0 {
                    break;
                }
                let mut ok = true;
                for &b in &chunk[..g] {
                    ok &= self.health[i].sample(b);
                }
                if !ok {
                    break;
                }
                dst[n..n + g].copy_from_slice(&chunk[..g]);
                n += g;
                got_here += g;
            }
            if self.health[i].failed {
                // Do not count anything from a source that failed during this draw (and do not leave it in dst).
                wipe(&mut dst[n - got_here..n]);
                n -= got_here;
            }
            if n >= want {
                break;
            }
        }
        wipe(&mut chunk);
        n
    }

    /// Jitter: hash 256 counter deltas around a little work. Mixed in, never counted.
    fn jitter<P: Platform>() -> [u8; 32] {
        let mut h = Sha256::new();
        let mut prev = P::counter();
        let mut acc = prev;
        for i in 0..256u64 {
            for j in 0..((acc & 7) + 1) {
                acc = acc.rotate_left(7) ^ j ^ i;
            }
            let now = P::counter();
            h.update(&now.wrapping_sub(prev).to_le_bytes());
            prev = now;
        }
        h.update(&acc.to_le_bytes());
        h.finish()
    }

    /// Gather one entropy input into the caller's `buf`: strong bytes (counted) || jitter hash || context.
    /// Returns (len, strong). The caller owns `buf` and wipes it once the seed is absorbed (F1); nothing that
    /// held entropy is returned by value, so no copy is left in this frame.
    fn gather<P: Platform>(&mut self, buf: &mut [u8; 128]) -> (usize, usize) {
        let strong = self.draw_strong::<P>(&mut buf[..SEED_STRONG_BYTES], SEED_STRONG_BYTES);
        let mut len = strong;
        let mut j = Self::jitter::<P>();
        buf[len..len + 32].copy_from_slice(&j);
        wipe(&mut j);
        len += 32;
        let mut ctx = [0u8; 24];
        P::context(&mut ctx);
        buf[len..len + 24].copy_from_slice(&ctx);
        wipe(&mut ctx);
        len += 24;
        (len, strong)
    }

    fn ensure<P: Platform>(&mut self) {
        if self.seeded {
            return;
        }
        if !self.kat_ran {
            self.kat_ok = kat();
            self.kat_ran = true;
        }
        self.probe_absent::<P>();
        let mut e = [0u8; 128];
        let mut n = [0u8; 128];
        let (el, s1) = self.gather::<P>(&mut e);
        let (nl, _) = self.gather::<P>(&mut n);
        let mut pers = [0u8; 32];
        let tag = b"AFRISOVERA-RNG-V1/";
        pers[..tag.len()].copy_from_slice(tag);
        let al = P::ARCH.len().min(32 - tag.len());
        pers[tag.len()..tag.len() + al].copy_from_slice(&P::ARCH[..al]);
        // Assigning drops (and so wipes) the previous state.
        self.drbg = HmacDrbg::instantiate(&e[..el], &n[..nl.min(48)], &pers[..tag.len() + al]);
        wipe(&mut e);
        wipe(&mut n);
        self.last_strong = s1;
        self.seeded = true;
    }

    /// Start-up test (90B 4.3) for every source not yet seen: 1024 samples through the health tests. Run at the
    /// first seed and again at every reseed, so a device that comes up after first use (virtio-rng is probed
    /// after early boot) is picked up later instead of being written off.
    fn probe_absent<P: Platform>(&mut self) {
        for i in 0..P::STRONG.len().min(MAX_SOURCES) {
            if self.present[i] {
                continue;
            }
            let mut probe = [0u8; 64];
            let g = P::strong(i, &mut probe);
            if g == 0 {
                continue;
            }
            self.present[i] = true;
            let mut tested = g;
            for &b in &probe[..g] {
                self.health[i].sample(b);
            }
            while tested < STARTUP_SAMPLES && !self.health[i].failed {
                let g = P::strong(i, &mut probe);
                if g == 0 {
                    self.health[i].failed = true; // a source that stops delivering is not healthy
                    break;
                }
                for &b in &probe[..g] {
                    self.health[i].sample(b);
                }
                tested += g;
            }
            wipe(&mut probe);
        }
    }

    fn reseed<P: Platform>(&mut self) -> usize {
        self.probe_absent::<P>();
        let mut e = [0u8; 128];
        let (el, s) = self.gather::<P>(&mut e);
        self.drbg.reseed(&e[..el], b"");
        wipe(&mut e);
        self.last_strong = s;
        self.reseeds += 1;
        s
    }

    /// Non-secret output (ids, ports, TCP ISS, uuids). Never fails.
    pub fn fill<P: Platform>(&mut self, out: &mut [u8]) {
        self.ensure::<P>();
        for chunk in out.chunks_mut(MAX_REQUEST) {
            if self.drbg.generate(chunk, b"").is_err() {
                self.reseed::<P>();
                // Cannot fail now: the counter was just reset and the chunk is within MAX_REQUEST.
                let _ = self.drbg.generate(chunk, b"");
            }
        }
    }

    /// Secret output (keys, nonces, ephemerals): KAT passed + fresh, health-tested strong entropy for THIS
    /// request, or an error and `out` zeroed.
    pub fn fill_secret<P: Platform>(&mut self, out: &mut [u8]) -> Result<(), RngError> {
        for b in out.iter_mut() {
            *b = 0;
        }
        if out.len() > MAX_REQUEST {
            self.secret_refused += 1;
            return Err(RngError::TooLong);
        }
        self.ensure::<P>();
        if !self.kat_ok {
            self.secret_refused += 1;
            return Err(RngError::KatFailed);
        }
        if self.reseed::<P>() < SEED_STRONG_BYTES {
            self.secret_refused += 1;
            return Err(RngError::NoStrongSource);
        }
        // Cannot fail: just reseeded and the length is checked above.
        if self.drbg.generate(out, b"").is_err() {
            wipe(out);
            self.secret_refused += 1;
            return Err(RngError::TooLong);
        }
        self.secret_ok += 1;
        Ok(())
    }

    pub fn status<P: Platform>(&mut self) -> Status {
        self.ensure::<P>();
        self.probe_absent::<P>();
        let mut s = Status {
            kat_ok: self.kat_ok,
            seeded: self.seeded,
            last_strong: self.last_strong,
            reseeds: self.reseeds,
            secret_ok: self.secret_ok,
            secret_refused: self.secret_refused,
            present: self.present,
            healthy: [false; MAX_SOURCES],
            samples: [0; MAX_SOURCES],
        };
        for i in 0..MAX_SOURCES {
            s.healthy[i] = self.present[i] && !self.health[i].failed;
            s.samples[i] = self.health[i].samples;
        }
        s
    }
}
