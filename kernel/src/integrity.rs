//! App integrity manifest — records the SHA-256 each installed app *should* have,
//! so AfriStore can detect a tampered or corrupted app before running it. The hash
//! is recorded at install/seed time (here, at boot); `verify` recomputes it and
//! compares. This is content-integrity, not authorship — asymmetric signing
//! (Ed25519) is future work (docs/12).

use crate::sha256;

// --- AfriStore code-signing (v0) ---
// The demo publisher's Ed25519 public key, and per-app signatures over each app's
// SHA-256 digest, produced off-device by scripts/sign-app.py. The device only
// verifies (Ed25519 verification, kernel/src/ed25519.rs). Real publishers ship
// their own key; this key is a reproducible demo key, not a secret.
#[rustfmt::skip]
pub const PUBLISHER_PUBKEY: [u8; 32] = [
    0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b, 0xa7,
    0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04, 0x96, 0x64,
];
#[rustfmt::skip]
static SUM_WASM_SIG: [u8; 64] = [
    0xfb, 0xfb, 0x7f, 0x6d, 0x4b, 0x7b, 0xf2, 0x57, 0x35, 0x92, 0x9e, 0x59, 0xce, 0xdd, 0xf1, 0x3b,
    0x17, 0x7b, 0x0f, 0x96, 0x2d, 0xb1, 0x05, 0xd5, 0xf8, 0x8b, 0x1e, 0x96, 0x83, 0x0d, 0x4a, 0xe7,
    0xe3, 0x20, 0xbc, 0xca, 0xbe, 0x98, 0xa4, 0x75, 0x02, 0x14, 0x2d, 0xff, 0x72, 0x8d, 0xff, 0xf6,
    0xd2, 0x5a, 0x0a, 0x31, 0xf6, 0x4c, 0x62, 0xa3, 0x42, 0xfd, 0x96, 0xbd, 0x4e, 0xf5, 0x86, 0x02,
];

/// Trust level of an installed app, for the store UI.
#[derive(Clone, Copy, PartialEq)]
pub enum Trust {
    Signed,     // a valid publisher Ed25519 signature over its digest
    Verified,   // hash matches the integrity manifest, but unsigned
    Unverified, // not in the manifest
}

/// Compute an app's trust level. Runs Ed25519 only for apps that carry a
/// signature (so an unsigned app is cheap), verifying over the app's SHA-256.
pub fn trust(name: &str, data: &[u8]) -> Trust {
    if let Some(sig) = signature_for(name) {
        let digest = sha256::digest(data);
        if crate::ed25519::verify(&PUBLISHER_PUBKEY, &digest, sig) {
            return Trust::Signed;
        }
    }
    match check(name, data) {
        Verdict::Ok => Trust::Verified,
        _ => Trust::Unverified,
    }
}

/// The publisher signature on file for `name` (over the app's SHA-256 digest), or
/// None if the app is not signed.
pub fn signature_for(name: &str) -> Option<&'static [u8; 64]> {
    match name {
        "sum.wasm" => Some(&SUM_WASM_SIG),
        _ => None,
    }
}

const MAX: usize = 16;
static mut NAMES: [&str; MAX] = [""; MAX];
static mut HASHES: [[u8; 32]; MAX] = [[0; 32]; MAX];
static mut COUNT: usize = 0;

// Scratch used only to demonstrate tamper detection (hash a modified copy).
const TCAP: usize = 8192;
static mut TBUF: [u8; TCAP] = [0; TCAP];

/// Record `name`'s expected digest (its bytes as installed).
pub fn record(name: &'static str, data: &[u8]) {
    // SAFETY: single-threaded bring-up; recorded before the shell runs.
    unsafe {
        if COUNT < MAX {
            NAMES[COUNT] = name;
            HASHES[COUNT] = sha256::digest(data);
            COUNT += 1;
        }
    }
}

/// Result of an integrity check at app-launch time.
#[derive(PartialEq)]
pub enum Verdict {
    Ok,         // bytes match the manifest — safe to run
    Unverified, // no manifest entry (e.g. a sideloaded/dev app) — allowed
    Tampered,   // manifest entry exists but the hash differs — must refuse
}

/// Check `data` for `name` against the manifest, for the launch-time gate.
pub fn check(name: &str, data: &[u8]) -> Verdict {
    match expected(name) {
        Some(exp) => {
            if sha256::digest(data) == exp {
                Verdict::Ok
            } else {
                Verdict::Tampered
            }
        }
        None => Verdict::Unverified,
    }
}

/// The expected digest for `name`, if it is in the manifest.
pub fn expected(name: &str) -> Option<[u8; 32]> {
    // SAFETY: scalar reads of data written once at boot.
    unsafe {
        for i in 0..COUNT {
            if NAMES[i] == name {
                return Some(HASHES[i]);
            }
        }
        None
    }
}

/// Digest of a *tampered* copy of `data` (first byte flipped) — for the `verify
/// <app> tamper` demo, showing the manifest catches a single changed byte.
pub fn hash_tampered(data: &[u8]) -> [u8; 32] {
    // SAFETY: single-threaded; TBUF accessed via raw pointer to avoid a &mut static.
    unsafe {
        let p = core::ptr::addr_of_mut!(TBUF) as *mut u8;
        let n = data.len().min(TCAP);
        for i in 0..n {
            *p.add(i) = data[i];
        }
        if n > 0 {
            *p.add(0) ^= 0xff; // flip one byte
        }
        sha256::digest(core::slice::from_raw_parts(p, n))
    }
}
