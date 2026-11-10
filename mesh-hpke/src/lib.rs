//! AFR-0009 sealing primitives via audited crates (plan 10.3): X25519 key agreement, Ed25519 strict
//! signing/verification, and ChaCha20-Poly1305 AEAD. Thin wrappers that fix the exact types the mesh uses;
//! the HPKE mode_auth key schedule and the bundle seal/open build on these next (reusing mesh-crypto's
//! validated HKDF/HMAC-SHA256).
//!
//! `ed25519_verify_strict` uses dalek's `verify_strict`, which rejects non-canonical `S` (the `S < L` check,
//! review 1 finding R1-6) and small-order public keys, so the strictness AFR-0009 section 3 requires is met.
//!
//! Host-validated against RFC 7748 section 6.1 (X25519), RFC 8032 TEST 1 (Ed25519) and RFC 8439 section 2.8.2
//! (ChaCha20-Poly1305). See tests.

#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use x25519_dalek::{PublicKey, StaticSecret};

/// X25519 public key for a (clamped) secret scalar.
pub fn x25519_public(secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

/// X25519 Diffie-Hellman shared secret.
pub fn x25519_dh(secret: &[u8; 32], peer_public: &[u8; 32]) -> [u8; 32] {
    StaticSecret::from(*secret).diffie_hellman(&PublicKey::from(*peer_public)).to_bytes()
}

/// Ed25519 public key for a secret seed.
pub fn ed25519_public(secret: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

/// Ed25519 signature over `msg`.
pub fn ed25519_sign(secret: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    SigningKey::from_bytes(secret).sign(msg).to_bytes()
}

/// Strict Ed25519 verification: canonical `S < L` and small-order keys rejected (review 1 R1-6).
pub fn ed25519_verify_strict(public: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    vk.verify_strict(msg, &Signature::from_bytes(sig)).is_ok()
}

/// Errors a caller can get. No panic path is reachable by a peer (review P6): inside the kernel a panic halts
/// the machine, so every fallible step returns instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// `bucket` was not 0..=3.
    BadBucket,
    /// The body does not fit the chosen bucket.
    BodyTooLong,
    /// A cryptographic step failed (seal, or a non-contributory X25519 result).
    Crypto,
}

/// ChaCha20-Poly1305 seal: returns ciphertext followed by the 16-byte tag, or `None` if the AEAD refuses
/// (never on valid inputs, but we do not panic - review P6).
pub fn aead_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(Nonce::from_slice(nonce), Payload { msg: plaintext, aad })
        .ok()
}

/// ChaCha20-Poly1305 open: `None` on authentication failure.
pub fn aead_open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad })
        .ok()
}

/// X25519 with the RFC 9180 "contributory" check (review S1): reject an all-zero shared secret (a low-order
/// peer key), returning `None` so the HPKE step fails cleanly instead of using a degenerate secret.
fn x25519_dh_checked(secret: &[u8; 32], peer: &[u8; 32]) -> Option<[u8; 32]> {
    let shared = StaticSecret::from(*secret).diffie_hellman(&PublicKey::from(*peer));
    if shared.was_contributory() {
        Some(shared.to_bytes())
    } else {
        None
    }
}

// =====================================================================================================
// HPKE (RFC 9180) mode_auth with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305 (AFR-0009 suite).
// The HKDF/HMAC-SHA256 come from mesh-crypto (milestone 1, vector-validated); the X25519 and AEAD come from the
// audited crates above. The labelled concatenations use Vec here (host crate); the no_std kernel port will use
// fixed stack buffers. mode_auth folds the sender's static key in, so the recipient tag authenticates the
// sender and no inner signature is needed (AFR-0009 section 6).

/// HPKE `info` for bundles (AFR-0009 section 3).
pub const CTX_BUNDLE: &[u8] = b"AFRISOVERA-MESH-BUNDLE-V1\0";

// "KEM" || u16be(0x0020) and "HPKE" || u16be(0x0020,0x0001,0x0003) (RFC 9180 suite ids).
const SUITE_KEM: &[u8] = &[0x4b, 0x45, 0x4d, 0x00, 0x20];
const SUITE: &[u8] = &[0x48, 0x50, 0x4b, 0x45, 0x00, 0x20, 0x00, 0x01, 0x00, 0x03];

fn labeled_extract(suite: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]) -> [u8; 32] {
    let mut v = Vec::with_capacity(7 + suite.len() + label.len() + ikm.len());
    v.extend_from_slice(b"HPKE-v1");
    v.extend_from_slice(suite);
    v.extend_from_slice(label);
    v.extend_from_slice(ikm);
    mesh_crypto::hkdf_extract(salt, &v)
}

fn labeled_expand(suite: &[u8], prk: &[u8; 32], label: &[u8], info: &[u8], l: usize) -> Vec<u8> {
    let mut li = Vec::new();
    li.extend_from_slice(&(l as u16).to_be_bytes());
    li.extend_from_slice(b"HPKE-v1");
    li.extend_from_slice(suite);
    li.extend_from_slice(label);
    li.extend_from_slice(info);
    let mut out = vec![0u8; l];
    mesh_crypto::hkdf_expand(prk, &li, &mut out);
    out
}

fn extract_and_expand(dh: &[u8], kem_context: &[u8]) -> [u8; 32] {
    let eae = labeled_extract(SUITE_KEM, &[], b"eae_prk", dh);
    let ss = labeled_expand(SUITE_KEM, &eae, b"shared_secret", kem_context, 32);
    let mut out = [0u8; 32];
    out.copy_from_slice(&ss);
    out
}

/// DHKEM AuthEncap: returns (shared_secret, enc) where `enc` is the sender's ephemeral public key. `None` if
/// either X25519 is non-contributory (review S1).
fn auth_encap(pk_r: &[u8; 32], sk_e: &[u8; 32], sk_s: &[u8; 32]) -> Option<([u8; 32], [u8; 32])> {
    let enc = x25519_public(sk_e);
    let mut dh = [0u8; 64];
    dh[..32].copy_from_slice(&x25519_dh_checked(sk_e, pk_r)?);
    dh[32..].copy_from_slice(&x25519_dh_checked(sk_s, pk_r)?);
    let pk_s = x25519_public(sk_s);
    let mut ctx = [0u8; 96];
    ctx[..32].copy_from_slice(&enc);
    ctx[32..64].copy_from_slice(pk_r);
    ctx[64..].copy_from_slice(&pk_s);
    Some((extract_and_expand(&dh, &ctx), enc))
}

/// DHKEM AuthDecap: the shared secret, given the sender's claimed static key `pk_s`. `None` if either X25519 is
/// non-contributory (review S1).
fn auth_decap(enc: &[u8; 32], sk_r: &[u8; 32], pk_s: &[u8; 32]) -> Option<[u8; 32]> {
    let mut dh = [0u8; 64];
    dh[..32].copy_from_slice(&x25519_dh_checked(sk_r, enc)?);
    dh[32..].copy_from_slice(&x25519_dh_checked(sk_r, pk_s)?);
    let pk_r = x25519_public(sk_r);
    let mut ctx = [0u8; 96];
    ctx[..32].copy_from_slice(enc);
    ctx[32..64].copy_from_slice(&pk_r);
    ctx[64..].copy_from_slice(pk_s);
    Some(extract_and_expand(&dh, &ctx))
}

fn key_schedule(mode: u8, ss: &[u8; 32], info: &[u8]) -> ([u8; 32], [u8; 12]) {
    let psk_id_hash = labeled_extract(SUITE, &[], b"psk_id_hash", &[]);
    let info_hash = labeled_extract(SUITE, &[], b"info_hash", info);
    let mut ctx = Vec::with_capacity(65);
    ctx.push(mode);
    ctx.extend_from_slice(&psk_id_hash);
    ctx.extend_from_slice(&info_hash);
    let secret = labeled_extract(SUITE, ss, b"secret", &[]);
    let keyv = labeled_expand(SUITE, &secret, b"key", &ctx, 32);
    let noncev = labeled_expand(SUITE, &secret, b"base_nonce", &ctx, 12);
    let mut key = [0u8; 32];
    key.copy_from_slice(&keyv);
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&noncev);
    (key, nonce)
}

/// HPKE mode_auth single-shot seal (sequence 0): returns (enc, ciphertext||tag). `None` on a non-contributory
/// X25519 or an AEAD refusal (review S1, P6).
pub fn hpke_auth_seal(pk_r: &[u8; 32], sk_e: &[u8; 32], sk_s: &[u8; 32], info: &[u8], aad: &[u8], pt: &[u8]) -> Option<([u8; 32], Vec<u8>)> {
    let (ss, enc) = auth_encap(pk_r, sk_e, sk_s)?;
    let (key, nonce) = key_schedule(0x02, &ss, info);
    Some((enc, aead_seal(&key, &nonce, aad, pt)?))
}

/// HPKE mode_auth single-shot open (sequence 0): `None` on authentication failure or a non-contributory X25519.
pub fn hpke_auth_open(enc: &[u8; 32], sk_r: &[u8; 32], pk_s: &[u8; 32], info: &[u8], aad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    let ss = auth_decap(enc, sk_r, pk_s)?;
    let (key, nonce) = key_schedule(0x02, &ss, info);
    aead_open(&key, &nonce, aad, ct)
}

// ---- AFR-0009 bundle (section 6) --------------------------------------------------------------------

/// Seal an AFR-0009 bundle (section 6). `bucket` is 0..=3; the output is exactly that bucket size.
/// `sk_s` is the sender's X25519 identity key, `pk_r` the recipient's.
#[allow(clippy::too_many_arguments)]
pub fn bundle_seal(
    bucket: usize,
    ttl: u8,
    bundle_id: &[u8; 16],
    tag: &[u8; 16],
    expires: u64,
    created: u64,
    mtype: u8,
    body: &[u8],
    sk_e: &[u8; 32],
    sk_s: &[u8; 32],
    pk_r: &[u8; 32],
) -> Result<Vec<u8>, Error> {
    // Review P1: a bad bucket or an over-long body is an error, not a silent truncation or a panic.
    if bucket >= mesh_crypto::BUCKETS.len() {
        return Err(Error::BadBucket);
    }
    let size = mesh_crypto::BUCKETS[bucket];
    let max_body = size - 92 - 12; // 152 / 920 / 3992 / 16280 (AFR-0009 section 6)
    if body.len() > max_body {
        return Err(Error::BodyTooLong);
    }
    // Inner plaintext (size - 92 bytes): type, reserved, u16 len, u64 created, body, then zero padding.
    let mut inner = Vec::with_capacity(size - 92);
    inner.push(mtype);
    inner.push(0);
    inner.extend_from_slice(&(body.len() as u16).to_le_bytes());
    inner.extend_from_slice(&created.to_le_bytes());
    inner.extend_from_slice(body);
    inner.resize(size - 92, 0);
    let hdr = mesh_crypto::bundle_header(ttl, bucket as u8, bundle_id, tag, expires);
    let aad = mesh_crypto::bundle_aad(&hdr);
    let (enc, ct) = hpke_auth_seal(pk_r, sk_e, sk_s, CTX_BUNDLE, &aad, &inner).ok_or(Error::Crypto)?;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&hdr);
    out.extend_from_slice(&enc);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open an AFR-0009 bundle as the recipient, given the sender's claimed X25519 key `pk_s` (found via the tag).
/// Returns (message type, created unix seconds, body). `None` on any authentication or parse failure. A relay's
/// decrement of the `ttl` byte is harmless because `ttl` is zeroed in the AAD.
pub fn bundle_open(bundle: &[u8], sk_r: &[u8; 32], pk_s: &[u8; 32]) -> Option<(u8, u64, Vec<u8>)> {
    if bundle.len() < 92 {
        return None;
    }
    // Review P2 (AFR-0009 section 6): version, and bucket matches the on-wire length, before anything else.
    if bundle[0] != 1 {
        return None;
    }
    let bucket = bundle[3] as usize;
    if bucket >= mesh_crypto::BUCKETS.len() || bundle.len() != mesh_crypto::BUCKETS[bucket] {
        return None;
    }
    let mut hdr = [0u8; 44];
    hdr.copy_from_slice(&bundle[..44]);
    let mut enc = [0u8; 32];
    enc.copy_from_slice(&bundle[44..76]);
    let ct = &bundle[76..];
    let aad = mesh_crypto::bundle_aad(&hdr);
    let pt = hpke_auth_open(&enc, sk_r, pk_s, CTX_BUNDLE, &aad, ct)?;
    if pt.len() < 12 || pt[1] != 0 {
        return None;
    }
    let mtype = pt[0];
    let len = u16::from_le_bytes([pt[2], pt[3]]) as usize;
    let created = u64::from_le_bytes(pt[4..12].try_into().ok()?);
    if 12 + len > pt.len() {
        return None;
    }
    // Review P2: the padding after the body must be all zero.
    if pt[12 + len..].iter().any(|&b| b != 0) {
        return None;
    }
    Some((mtype, created, pt[12..12 + len].to_vec()))
}

// =====================================================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
    fn arr32(s: &str) -> [u8; 32] {
        let mut a = [0u8; 32];
        a.copy_from_slice(&unhex(s));
        a
    }
    #[test]
    fn x25519_rfc7748_6_1() {
        let a = arr32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let b = arr32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        assert_eq!(
            x25519_public(&a).to_vec(),
            unhex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        assert_eq!(
            x25519_public(&b).to_vec(),
            unhex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        );
        let shared = unhex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(x25519_dh(&a, &x25519_public(&b)).to_vec(), shared);
        assert_eq!(x25519_dh(&b, &x25519_public(&a)).to_vec(), shared, "DH must agree both ways");
    }

    #[test]
    fn ed25519_rfc8032_test1() {
        let secret = arr32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let public = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        assert_eq!(ed25519_public(&secret).to_vec(), unhex(public));
        // RFC 8032 TEST 1: empty message.
        let sig = ed25519_sign(&secret, b"");
        assert_eq!(
            sig.to_vec(),
            unhex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
        );
        assert!(ed25519_verify_strict(&arr32(public), b"", &sig));
        // Wrong message must fail.
        assert!(!ed25519_verify_strict(&arr32(public), b"x", &sig));
        // A mangled S must fail strict verification (R1-6 territory).
        let mut bad = sig;
        bad[63] ^= 0x01;
        assert!(!ed25519_verify_strict(&arr32(public), b"", &bad));
    }

    #[test]
    fn chacha20poly1305_rfc8439_2_8_2() {
        let key = arr32("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&unhex("070000004041424344454647"));
        let aad = unhex("50515253c0c1c2c3c4c5c6c7");
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let want = unhex(concat!(
            "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d6",
            "3dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b36",
            "92ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc",
            "3ff4def08e4b7a9de576d26586cec64b6116",
            "1ae10b594f09e26a7e902ecbd0600691"
        ));
        // `want` is the RFC 8439 section 2.8.2 output, independently reproduced by OpenSSL's
        // ChaCha20-Poly1305 for these exact inputs (cross-implementation check against the RustCrypto crate).
        let ct = aead_seal(&key, &nonce, &aad, pt).unwrap();
        assert_eq!(ct, want, "RFC 8439 ciphertext||tag");
        assert_eq!(aead_open(&key, &nonce, &aad, &ct).unwrap(), pt.to_vec());
        // Tamper the tag: open must fail.
        let mut bad = ct.clone();
        *bad.last_mut().unwrap() ^= 0x01;
        assert!(aead_open(&key, &nonce, &aad, &bad).is_none());
        // Wrong AAD: open must fail.
        assert!(aead_open(&key, &nonce, b"wrong", &ct).is_none());
    }

    #[test]
    fn bundle_matches_architect_vector_and_opens_after_relay() {
        // Alice -> Bob, bucket 0, text "Habari, Bob!", ttl 8, expires T+24h, fixed ephemeral skE = 0x11*32.
        let a = arr32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"); // xA secret
        let b_sec = arr32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"); // xB secret
        let pa = x25519_public(&a);
        let pb = x25519_public(&b_sec);
        let dh = x25519_dh(&a, &pb);
        let root = mesh_crypto::contact_root(&pa, &pb, &dh);
        let period = mesh_crypto::tag_period(1794225600);
        let tag = mesh_crypto::recipient_tag(&root, &pa, &pb, period);
        let ske = [0x11u8; 32];
        let mut bid = [0u8; 16];
        for i in 0..16 {
            bid[i] = i as u8;
        }
        let created = 1794225600u64;
        let bundle = bundle_seal(0, 8, &bid, &tag, created + 86400, created, 1, b"Habari, Bob!", &ske, &a, &pb).expect("seal");
        assert_eq!(bundle.len(), 256, "bucket 0 is 256 bytes");
        // Whole-bundle check against the architect's AFR-0009 bundle vector (sha256 over the 256 bytes).
        assert_eq!(
            mesh_crypto::sha256::digest(&bundle).to_vec(),
            unhex("fbff0cc1a6fbe3838eda2a58b06adaa25a7b15f3df5a71cf84969783248d4bb4")
        );
        // A relay decrements ttl (byte 2); it is zeroed in the AAD, so the recipient still opens the bundle.
        let mut relayed = bundle.clone();
        relayed[2] = 7;
        let (mtype, got_created, body) = bundle_open(&relayed, &b_sec, &pa).expect("recipient opens after relay");
        assert_eq!((mtype, got_created, body), (1u8, created, b"Habari, Bob!".to_vec()));
        // A different claimed sender (Bob's own key) must fail: mode_auth binds the sender.
        assert!(bundle_open(&relayed, &b_sec, &pb).is_none(), "wrong sender must be rejected");

        // Review P2 reject tests (AFR-0009 section 6 open-time checks).
        let mut bad_ver = bundle.clone();
        bad_ver[0] = 2;
        assert!(bundle_open(&bad_ver, &b_sec, &pa).is_none(), "wrong version rejected");
        let mut bad_bucket = bundle.clone();
        bad_bucket[3] = 1; // claims bucket 1 (1024 bytes) but the frame is 256
        assert!(bundle_open(&bad_bucket, &b_sec, &pa).is_none(), "bucket/length mismatch rejected");
        let mut tampered = bundle.clone();
        *tampered.last_mut().unwrap() ^= 0x01; // corrupt the ciphertext tag
        assert!(bundle_open(&tampered, &b_sec, &pa).is_none(), "tampered ciphertext rejected");
    }

    #[test]
    fn bundle_seal_rejects_bad_inputs() {
        // Review P1: a bad bucket and an over-long body are errors, not silent truncation or a panic.
        let a = arr32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let pb = x25519_public(&arr32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"));
        let ske = [0x11u8; 32];
        let bid = [0u8; 16];
        let tag = [0u8; 16];
        assert_eq!(bundle_seal(4, 8, &bid, &tag, 0, 0, 1, b"x", &ske, &a, &pb), Err(Error::BadBucket));
        let too_long = [0u8; 153]; // bucket 0 max body is 152
        assert_eq!(bundle_seal(0, 8, &bid, &tag, 0, 0, 1, &too_long, &ske, &a, &pb), Err(Error::BodyTooLong));
        // The largest exact body for bucket 0 is accepted.
        let max_body = [0u8; 152];
        assert!(bundle_seal(0, 8, &bid, &tag, 0, 0, 1, &max_body, &ske, &a, &pb).is_ok());
    }
}
