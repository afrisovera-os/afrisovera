//! The Wi-Fi stack's cryptographic primitives (W1). THE ONLY FILE in the kernel that names `wlan_crypto`, the path
//! crate over the pinned RustCrypto crates (ADR 0007 row "Wi-Fi primitives", architect's verdict 14 Nov 2026; the
//! wifi-crypto suite fails if any other file names it or the crates behind it).
//!
//! What 802.11 needs and nothing else: HMAC-SHA1, PBKDF2 (passphrase to PMK), the 802.11 PRF, AES key wrap (the
//! group key), AES-CCM with an 8-byte tag (CCMP) and AES-CMAC. Software AES only: the PC kernel saves no SIMD state
//! (`aes_force_soft`, .cargo/config.toml; checked by disassembly in the suite).
//!
//! Rules kept here:
//! - Known-answer tests: `selftest` runs every published vector in crypto_kat.rs. Until it has passed in this boot,
//!   `kat_passed()` is false, and the Wi-Fi commands that need keys refuse (the architect's condition).
//! - Secrets live in [`Secret`], which wipes itself with volatile writes and a compiler fence when dropped, so the
//!   compiler cannot drop the wipe. [`wipe`] does the same for a caller's buffer (the passphrase input line).
//! - Every comparison of a MIC or tag is constant time.
#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

pub use wlan_crypto::{Bad, PMK_LEN, PSK_ROUNDS, SHA1_LEN};

/// Overwrite `b` with zeros in a way the compiler may not remove (volatile writes, then a compiler fence).
pub fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: a valid, aligned &mut u8.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
}

/// A fixed-size secret (PMK, PTK, GTK, a passphrase copy) that is wiped when it goes out of scope.
pub struct Secret<const N: usize>(pub [u8; N]);

impl<const N: usize> Secret<N> {
    pub const fn zero() -> Self {
        Secret([0u8; N])
    }
    pub fn wipe(&mut self) {
        wipe(&mut self.0);
    }
}

impl<const N: usize> Drop for Secret<N> {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

pub fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> [u8; SHA1_LEN] {
    wlan_crypto::hmac_sha1(key, parts)
}

/// Constant time; `tag` may be a left truncation (802.11's MIC is the first 16 bytes of HMAC-SHA1).
pub fn hmac_sha1_verify(key: &[u8], parts: &[&[u8]], tag: &[u8]) -> bool {
    wlan_crypto::hmac_sha1_verify(key, parts, tag)
}

pub fn pbkdf2_sha1(password: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) -> Result<(), Bad> {
    wlan_crypto::pbkdf2_sha1(password, salt, rounds, out)
}

/// The PMK of WPA2-Personal from a passphrase (8 to 63 printable ASCII) and the SSID (0 to 32 bytes).
pub fn pmk_from_passphrase(passphrase: &[u8], ssid: &[u8]) -> Result<Secret<PMK_LEN>, Bad> {
    let mut k = wlan_crypto::psk_from_passphrase(passphrase, ssid)?;
    let s = Secret(k);
    wipe(&mut k);
    Ok(s)
}

pub fn prf_sha1(key: &[u8], label: &[u8], data: &[u8], out: &mut [u8]) -> Result<(), Bad> {
    wlan_crypto::prf_sha1(key, label, data, out)
}

pub fn aes_wrap(kek: &[u8; 16], plain: &[u8], out: &mut [u8]) -> Result<(), Bad> {
    wlan_crypto::aes_wrap(kek, plain, out)
}

/// Refused, and `out` zeroed, when the integrity check value does not match.
pub fn aes_unwrap(kek: &[u8; 16], wrapped: &[u8], out: &mut [u8]) -> Result<(), Bad> {
    wlan_crypto::aes_unwrap(kek, wrapped, out)
}

pub fn ccm8_encrypt(key: &[u8; 16], nonce: &[u8; 13], aad: &[u8], buf: &mut [u8]) -> Result<[u8; 8], Bad> {
    wlan_crypto::ccm8_encrypt(key, nonce, aad, buf)
}

/// Refused, and `buf` zeroed, on a wrong tag: no unauthenticated plaintext is ever released.
pub fn ccm8_decrypt(key: &[u8; 16], nonce: &[u8; 13], aad: &[u8], buf: &mut [u8], tag: &[u8; 8]) -> Result<(), Bad> {
    wlan_crypto::ccm8_decrypt(key, nonce, aad, buf, tag)
}

pub fn cmac_aes128(key: &[u8; 16], parts: &[&[u8]]) -> [u8; 16] {
    wlan_crypto::cmac_aes128(key, parts)
}

pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    wlan_crypto::ct_eq(a, b)
}

static KAT_OK: AtomicBool = AtomicBool::new(false);

/// True once [`selftest`] has passed in this boot.
pub fn kat_passed() -> bool {
    KAT_OK.load(Ordering::Relaxed)
}

/// Which family of vectors; `selftest` reports per family.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    HmacSha1,
    Pbkdf2Sha1,
    Psk,
    PrfSha1,
    AesKw,
    Cmac,
    Ccm8,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::HmacSha1 => "HMAC-SHA1 (RFC 2202)",
            Kind::Pbkdf2Sha1 => "PBKDF2-HMAC-SHA1 (RFC 6070)",
            Kind::Psk => "passphrase to PMK (802.11 Annex J.4)",
            Kind::PrfSha1 => "802.11 PRF-SHA1 (Annex J.3)",
            Kind::AesKw => "AES key wrap (RFC 3394)",
            Kind::Cmac => "AES-CMAC (RFC 4493)",
            Kind::Ccm8 => "AES-CCM, 8-byte tag (RFC 3610)",
        }
    }
}

pub const KINDS: [Kind; 7] = [Kind::HmacSha1, Kind::Pbkdf2Sha1, Kind::Psk, Kind::PrfSha1, Kind::AesKw, Kind::Cmac, Kind::Ccm8];

/// Run every vector of one family. Ok(count) or Err(index of the first vector that failed).
pub fn run_kind(k: Kind) -> Result<usize, usize> {
    use super::crypto_kat as v;
    let mut scratch = [0u8; 128];
    match k {
        Kind::HmacSha1 => {
            for (i, &(key, data, want)) in v::HMAC_SHA1.iter().enumerate() {
                if hmac_sha1(key, &[data])[..] != *want || !hmac_sha1_verify(key, &[data], want) {
                    return Err(i);
                }
            }
            Ok(v::HMAC_SHA1.len())
        }
        Kind::Pbkdf2Sha1 => {
            for (i, &(p, s, c, want)) in v::PBKDF2_SHA1.iter().enumerate() {
                let out = scratch.get_mut(..want.len()).ok_or(i)?;
                if pbkdf2_sha1(p, s, c, out).is_err() || out != want {
                    return Err(i);
                }
            }
            Ok(v::PBKDF2_SHA1.len())
        }
        Kind::Psk => {
            for (i, &(p, s, want)) in v::PSK.iter().enumerate() {
                match pmk_from_passphrase(p, s) {
                    Ok(k) if k.0[..] == *want => {}
                    _ => return Err(i),
                }
            }
            Ok(v::PSK.len())
        }
        Kind::PrfSha1 => {
            for (i, &(key, label, data, want)) in v::PRF_SHA1.iter().enumerate() {
                let out = scratch.get_mut(..want.len()).ok_or(i)?;
                if prf_sha1(key, label, data, out).is_err() || out != want {
                    return Err(i);
                }
            }
            Ok(v::PRF_SHA1.len())
        }
        Kind::AesKw => {
            for (i, &(kek, plain, ct)) in v::AES_KW.iter().enumerate() {
                let kek: &[u8; 16] = kek.try_into().map_err(|_| i)?;
                let (a, b) = scratch.split_at_mut(64);
                let w = a.get_mut(..plain.len() + 8).ok_or(i)?;
                let u = b.get_mut(..plain.len()).ok_or(i)?;
                if aes_wrap(kek, plain, w).is_err() || w != ct || aes_unwrap(kek, ct, u).is_err() || u != plain {
                    return Err(i);
                }
                // one changed bit must be refused
                w[0] ^= 1;
                if aes_unwrap(kek, w, u).is_ok() {
                    return Err(i);
                }
            }
            Ok(v::AES_KW.len())
        }
        Kind::Cmac => {
            for (i, &(key, msg, want)) in v::CMAC.iter().enumerate() {
                let key: &[u8; 16] = key.try_into().map_err(|_| i)?;
                if cmac_aes128(key, &[msg])[..] != *want {
                    return Err(i);
                }
            }
            Ok(v::CMAC.len())
        }
        Kind::Ccm8 => {
            for (i, &(key, nonce, aad, plain, ct)) in v::CCM8.iter().enumerate() {
                let key: &[u8; 16] = key.try_into().map_err(|_| i)?;
                let nonce: &[u8; 13] = nonce.try_into().map_err(|_| i)?;
                if ct.len() != plain.len() + 8 {
                    return Err(i);
                }
                let buf = scratch.get_mut(..plain.len()).ok_or(i)?;
                buf.copy_from_slice(plain);
                let tag = ccm8_encrypt(key, nonce, aad, buf).map_err(|_| i)?;
                if buf != &ct[..plain.len()] || tag[..] != ct[plain.len()..] {
                    return Err(i);
                }
                if ccm8_decrypt(key, nonce, aad, buf, &tag).is_err() || buf != plain {
                    return Err(i);
                }
                // a changed tag must be refused, and nothing released
                let mut bad = tag;
                bad[7] ^= 0x80;
                buf.copy_from_slice(&ct[..plain.len()]);
                if ccm8_decrypt(key, nonce, aad, buf, &bad).is_ok() || buf.iter().any(|&b| b != 0) {
                    return Err(i);
                }
            }
            Ok(v::CCM8.len())
        }
    }
}

/// Mark the known-answer tests as passed or failed for this boot (called by the shell after running every kind).
pub fn set_kat_result(ok: bool) {
    KAT_OK.store(ok, Ordering::Relaxed);
}
