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
// Re-signed for C1 (sum.wasm now carries its permission list): scripts/sign-app.py WASM_SUM.
#[rustfmt::skip]
static SUM_WASM_SIG: [u8; 64] = [
    0xbc, 0xa9, 0x6b, 0x39, 0xce, 0x97, 0x48, 0xc1, 0xee, 0x57, 0xcc, 0xa2, 0x3b, 0x13, 0x8d, 0xb9,
    0x10, 0x91, 0x4f, 0x62, 0x35, 0x0d, 0x22, 0xc9, 0x3c, 0x6d, 0x93, 0x77, 0x93, 0x18, 0x77, 0x3b,
    0x15, 0xd5, 0x21, 0xa5, 0xc5, 0x0e, 0x9f, 0x82, 0x20, 0x52, 0x85, 0x05, 0xe8, 0xd4, 0x2a, 0x7c,
    0x53, 0xd5, 0x75, 0x7b, 0x10, 0xb9, 0xbf, 0x66, 0x00, 0xd5, 0x21, 0xd1, 0xfb, 0xda, 0xa9, 0x02,
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

const MAX: usize = 32; // 16 until A2: the bundled apps grow (Andika, Ngoma, Hati)
static mut NAMES: [&str; MAX] = [""; MAX];
static mut HASHES: [[u8; 32]; MAX] = [[0; 32]; MAX];
static mut COUNT: usize = 0;
static mut REFUSED: usize = 0;

/// Record `name`'s expected digest (its bytes as installed).
pub fn record(name: &'static str, data: &[u8]) {
    // SAFETY: single-threaded bring-up; recorded before the shell runs.
    unsafe {
        if COUNT < MAX {
            NAMES[COUNT] = name;
            HASHES[COUNT] = sha256::digest(data);
            COUNT += 1;
        } else {
            // Never a silent drop (the architect's A7 change 3): the app would run unverified.
            REFUSED += 1;
            crate::kprintln!("[integrity] manifest full ({} entries): {} not recorded", MAX, name);
        }
    }
}

/// How many `record` calls the full manifest refused.
pub fn refused() -> usize {
    // SAFETY: a scalar read of data written at boot.
    unsafe { REFUSED }
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
    // Streamed, so an app of any size is hashed whole (A7: a bundled app may be over 16 KiB) with no copy.
    let mut h = sha256::Sha256::new();
    if let Some((&first, rest)) = data.split_first() {
        h.update(&[first ^ 0xff]); // flip one byte
        h.update(rest);
    }
    h.finish()
}
