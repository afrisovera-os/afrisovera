//! Wi-Fi (WPA2-Personal) primitives for the AfriSovera PC kernel (W1 of docs: WIFI_PLAN.md; ADR 0007 row
//! "Wi-Fi primitives", architect's verdict 14 Nov 2026).
//!
//! Thin, core-only wrappers over pinned RustCrypto crates. Each function fixes the parameters 802.11 uses, takes
//! slices, and returns a plain `Result` instead of panicking on a bad length. No heap. The protocol logic (EAPOL,
//! the key hierarchy, the handshakes, CCMP framing, nonces) is not here: it is our own code in kernel/src/wlan/.
//!
//! Secrets: the wrappers keep no state. Callers wipe their own key buffers (kernel/src/wlan/crypto.rs does, with
//! volatile writes and a compiler fence). The crates' internal states are on the stack and are not wiped by us.
#![no_std]
#![forbid(unsafe_code)]

// The PC kernel saves no SIMD state, so the AES-NI path of `aes` must never be compiled in (architect, 14 Nov).
#[cfg(all(target_os = "uefi", not(aes_force_soft)))]
compile_error!("wlan-crypto for UEFI needs --cfg aes_force_soft (set in .cargo/config.toml for x86_64-unknown-uefi)");

use aes::cipher::KeyInit;
use aes::Aes128;
use ccm::aead::AeadInPlace;
use ccm::consts::{U13, U8};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;

/// Something the caller gave was the wrong length or shape, or an integrity check failed. Deliberately no detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bad;

pub const SHA1_LEN: usize = 20;
/// The WPA2 PSK / PMK length.
pub const PMK_LEN: usize = 32;
/// PBKDF2 rounds for a WPA passphrase (IEEE 802.11 Annex J.4).
pub const PSK_ROUNDS: u32 = 4096;

type HmacSha1 = Hmac<Sha1>;
type Ccm8 = ccm::Ccm<Aes128, U8, U13>;

/// HMAC-SHA1 over the concatenation of `parts`.
pub fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> [u8; SHA1_LEN] {
    // new_from_slice accepts any key length for HMAC (keys longer than a block are hashed first)
    let mut m = <HmacSha1 as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// True if `tag` (1 to 20 bytes) equals the leftmost bytes of HMAC-SHA1(key, parts), compared in constant time.
pub fn hmac_sha1_verify(key: &[u8], parts: &[&[u8]], tag: &[u8]) -> bool {
    if tag.is_empty() || tag.len() > SHA1_LEN {
        return false;
    }
    let full = hmac_sha1(key, parts);
    ct_eq(&full[..tag.len()], tag)
}

/// PBKDF2-HMAC-SHA1 (RFC 8018) of `password` and `salt` with `rounds`, filling `out`.
pub fn pbkdf2_sha1(password: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) -> Result<(), Bad> {
    if rounds == 0 || out.is_empty() {
        return Err(Bad);
    }
    pbkdf2::pbkdf2::<HmacSha1>(password, salt, rounds, out).map_err(|_| Bad)
}

/// The WPA2 PSK (the PMK of WPA2-Personal) from a passphrase and the network name (IEEE 802.11 Annex J.4).
/// The passphrase must be 8 to 63 printable ASCII characters and the SSID 0 to 32 bytes; anything else is refused.
/// (A 64-hex-digit PSK typed instead of a passphrase is decoded by the caller, not here.)
pub fn psk_from_passphrase(passphrase: &[u8], ssid: &[u8]) -> Result<[u8; PMK_LEN], Bad> {
    if !(8..=63).contains(&passphrase.len()) || !passphrase.iter().all(|&c| (0x20..=0x7E).contains(&c)) {
        return Err(Bad);
    }
    if ssid.len() > 32 {
        return Err(Bad);
    }
    let mut out = [0u8; PMK_LEN];
    pbkdf2_sha1(passphrase, ssid, PSK_ROUNDS, &mut out)?;
    Ok(out)
}

/// The IEEE 802.11 PRF built on HMAC-SHA1 (802.11-2020 section 12.7.1.2): `out` is filled with the leftmost bytes
/// of HMAC-SHA1(key, label || 0x00 || data || i) for i = 0, 1, 2, and so on. At most 255 blocks (5100 bytes).
pub fn prf_sha1(key: &[u8], label: &[u8], data: &[u8], out: &mut [u8]) -> Result<(), Bad> {
    if out.is_empty() || out.len() > 255 * SHA1_LEN {
        return Err(Bad);
    }
    for (i, chunk) in out.chunks_mut(SHA1_LEN).enumerate() {
        let block = hmac_sha1(key, &[label, &[0u8], data, &[i as u8]]);
        chunk.copy_from_slice(&block[..chunk.len()]);
    }
    Ok(())
}

/// AES key wrap (RFC 3394) with a 128-bit KEK: `plain` is a multiple of 8 bytes, at least 16; `out` is 8 bytes
/// longer than `plain`.
pub fn aes_wrap(kek: &[u8; 16], plain: &[u8], out: &mut [u8]) -> Result<(), Bad> {
    if plain.len() < 16 || plain.len() % 8 != 0 || out.len() != plain.len() + 8 {
        return Err(Bad);
    }
    aes_kw::KekAes128::new(kek.into()).wrap(plain, out).map_err(|_| Bad)
}

/// AES key unwrap (RFC 3394) with a 128-bit KEK: `wrapped` is a multiple of 8 bytes, at least 24; `out` is 8
/// bytes shorter. Refused (and `out` zeroed) when the integrity check value does not match.
pub fn aes_unwrap(kek: &[u8; 16], wrapped: &[u8], out: &mut [u8]) -> Result<(), Bad> {
    if wrapped.len() < 24 || wrapped.len() % 8 != 0 || out.len() + 8 != wrapped.len() {
        return Err(Bad);
    }
    let r = aes_kw::KekAes128::new(kek.into()).unwrap(wrapped, out);
    if r.is_err() {
        out.iter_mut().for_each(|b| *b = 0);
        return Err(Bad);
    }
    Ok(())
}

/// AES-128-CCM with an 8-byte tag and a 13-byte nonce (CCMP's parameters): encrypts `buf` in place, returns the tag.
pub fn ccm8_encrypt(key: &[u8; 16], nonce: &[u8; 13], aad: &[u8], buf: &mut [u8]) -> Result<[u8; 8], Bad> {
    let c = Ccm8::new(key.into());
    let tag = c.encrypt_in_place_detached(nonce.into(), aad, buf).map_err(|_| Bad)?;
    Ok(tag.into())
}

/// AES-128-CCM decryption with an 8-byte tag and a 13-byte nonce, in place. On a wrong tag the result is refused
/// and `buf` is zeroed, so no unauthenticated plaintext ever leaves this function.
pub fn ccm8_decrypt(key: &[u8; 16], nonce: &[u8; 13], aad: &[u8], buf: &mut [u8], tag: &[u8; 8]) -> Result<(), Bad> {
    let c = Ccm8::new(key.into());
    if c.decrypt_in_place_detached(nonce.into(), aad, buf, tag.into()).is_err() {
        buf.iter_mut().for_each(|b| *b = 0);
        return Err(Bad);
    }
    Ok(())
}

/// AES-128-CMAC (RFC 4493) over the concatenation of `parts`.
pub fn cmac_aes128(key: &[u8; 16], parts: &[&[u8]]) -> [u8; 16] {
    let mut m = <cmac::Cmac<Aes128> as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// Constant-time equality of two byte strings (false at once if the lengths differ; lengths are not secret).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrase_rules() {
        assert_eq!(psk_from_passphrase(b"short", b"x"), Err(Bad));
        assert_eq!(psk_from_passphrase(&[b'a'; 64], b"x"), Err(Bad));
        assert_eq!(psk_from_passphrase(b"has\ttab1", b"x"), Err(Bad));
        assert_eq!(psk_from_passphrase(b"goodpass", &[0u8; 33]), Err(Bad));
        assert!(psk_from_passphrase(b"goodpass", &[0u8; 32]).is_ok());
        assert!(psk_from_passphrase(&[b'~'; 63], b"").is_ok());
    }

    #[test]
    fn lengths_refused_not_panicking() {
        let k = [0u8; 16];
        let mut o = [0u8; 24];
        assert_eq!(aes_wrap(&k, &[0u8; 8], &mut o[..16]), Err(Bad));
        assert_eq!(aes_wrap(&k, &[0u8; 17], &mut o), Err(Bad));
        assert_eq!(aes_unwrap(&k, &[0u8; 16], &mut o[..8]), Err(Bad));
        assert_eq!(aes_unwrap(&k, &[0u8; 25], &mut o[..17]), Err(Bad));
        assert_eq!(prf_sha1(&k, b"l", b"d", &mut []), Err(Bad));
        let mut big = [0u8; 255 * 20 + 1];
        assert_eq!(prf_sha1(&k, b"l", b"d", &mut big), Err(Bad));
        assert_eq!(pbkdf2_sha1(b"p", b"s", 0, &mut o), Err(Bad));
        assert!(!hmac_sha1_verify(&k, &[b"x"], &[]));
        assert!(!hmac_sha1_verify(&k, &[b"x"], &[0u8; 21]));
    }

    #[test]
    fn unwrap_tamper_zeroes_output() {
        let kek = [7u8; 16];
        let plain = [0x11u8; 16];
        let mut w = [0u8; 24];
        aes_wrap(&kek, &plain, &mut w).unwrap();
        let mut o = [0xEEu8; 16];
        aes_unwrap(&kek, &w, &mut o).unwrap();
        assert_eq!(o, plain);
        w[5] ^= 1;
        let mut o = [0xEEu8; 16];
        assert_eq!(aes_unwrap(&kek, &w, &mut o), Err(Bad));
        assert_eq!(o, [0u8; 16]);
    }

    #[test]
    fn ccm_tamper_refused_and_zeroed() {
        let key = [3u8; 16];
        let nonce = [9u8; 13];
        let aad = [1u8, 2, 3];
        let mut buf = *b"sixteen byte msg";
        let tag = ccm8_encrypt(&key, &nonce, &aad, &mut buf).unwrap();
        let ct = buf;
        // every single-bit change of AAD, nonce, ciphertext or tag is refused, and nothing is released
        for which in 0..4 {
            for bit in 0..8 {
                let (mut a, mut n, mut c, mut t) = (aad, nonce, ct, tag);
                match which {
                    0 => a[1] ^= 1 << bit,
                    1 => n[12] ^= 1 << bit,
                    2 => c[7] ^= 1 << bit,
                    _ => t[0] ^= 1 << bit,
                }
                assert_eq!(ccm8_decrypt(&key, &n, &a, &mut c, &t), Err(Bad));
                assert_eq!(c, [0u8; 16]);
            }
        }
        let mut c = ct;
        ccm8_decrypt(&key, &nonce, &aad, &mut c, &tag).unwrap();
        assert_eq!(&c, b"sixteen byte msg");
    }
}
