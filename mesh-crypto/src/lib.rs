//! AFR-0009 (Sovereign Mesh wire format) derivation + wire-format layer.
//!
//! Scope of THIS module (plan of record, friction-free first milestone): the parts of AFR-0009 that stand on
//! SHA-256 alone and need no side-channel-sensitive primitive:
//!   - HMAC-SHA256 (FIPS 198) and HKDF-SHA256 (RFC 5869);
//!   - contact root (spec 5.3), recipient tag (5.4), beacon tag (5.5), short authentication string (4.3);
//!   - identity-record layout (5.2) and bundle header + AAD layout (6).
//!
//! NOT here, by design: X25519 and ChaCha20-Poly1305 and HPKE. Those are the audited-crate primitives of the
//! next step (plan 10.3). Where a derivation needs the X25519 shared secret (the contact root), it is taken as
//! an input (`dh`), so this module stays primitive-free and the crate choice is not pre-empted.
//!
//! The library code is written core-only (no heap, no std items: only arrays, slices and integer ops), so it
//! drops into the kernel's future `kernel/src/mesh/` tree unchanged. The crate is `#![no_std]` for every build
//! except the test build (which needs std for the harness), so it compiles for the bare-metal kernel target
//! `aarch64-unknown-none` (verified: `cargo build --manifest-path mesh-crypto/Cargo.toml --target
//! aarch64-unknown-none`). Host-validated against `docs/spec/vectors/afr0009_vectors.py` (see tests); run tests
//! with `--target <host triple>` because the repo default target is bare-metal.

#![cfg_attr(not(test), no_std)]

pub mod sha256;
use sha256::{digest, Sha256};

// AFR-0009 section 3 context strings (ASCII + trailing NUL).
pub const CTX_CONTACT: &[u8] = b"AFRISOVERA-MESH-CONTACT-V1\0";
pub const CTX_TAG: &[u8] = b"AFRISOVERA-MESH-TAG-V1\0";
pub const CTX_BEACON: &[u8] = b"AFRISOVERA-MESH-BEACON-V1\0";
pub const CTX_SAS: &[u8] = b"AFRISOVERA-MESH-SAS-V1\0";
pub const CTX_IDENTITY: &[u8] = b"AFRISOVERA-MESH-IDENTITY-V1\0";

/// Bundle size buckets (spec 6).
pub const BUCKETS: [usize; 4] = [256, 1024, 4096, 16384];

// ---- HMAC-SHA256 (FIPS 198) --------------------------------------------------------------------------

/// HMAC-SHA256 over the concatenation of `parts`, so callers never need a heap to join a message.
pub fn hmac_parts(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut k0 = [0u8; 64];
    if key.len() > 64 {
        k0[..32].copy_from_slice(&digest(key));
    } else {
        k0[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k0[i] ^ 0x36;
        opad[i] = k0[i] ^ 0x5c;
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    for p in parts {
        inner.update(p);
    }
    let inner = inner.finish();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner);
    outer.finish()
}

/// HMAC-SHA256 over a single message.
pub fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    hmac_parts(key, &[msg])
}

// ---- HKDF-SHA256 (RFC 5869) --------------------------------------------------------------------------

/// HKDF-Extract: an empty salt is treated as 32 zero bytes (RFC 5869 section 2.2).
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    if salt.is_empty() {
        hmac_parts(&[0u8; 32], &[ikm])
    } else {
        hmac_parts(salt, &[ikm])
    }
}

/// HKDF-Expand into `out` (RFC 5869 section 2.3). `out.len()` must be <= 255*32.
pub fn hkdf_expand(prk: &[u8; 32], info: &[u8], out: &mut [u8]) {
    let mut t = [0u8; 32];
    let mut t_len = 0usize;
    let mut i = 1u8;
    let mut off = 0usize;
    while off < out.len() {
        let block = hmac_parts(prk, &[&t[..t_len], info, &[i]]);
        let n = core::cmp::min(32, out.len() - off);
        out[off..off + n].copy_from_slice(&block[..n]);
        t = block;
        t_len = 32;
        off += n;
        i = i.wrapping_add(1);
    }
}

/// HKDF = Extract then Expand, into `out`.
pub fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) {
    let prk = hkdf_extract(salt, ikm);
    hkdf_expand(&prk, info, out);
}

// ---- AFR-0009 derivations ---------------------------------------------------------------------------

/// Contact root (spec 5.3). `dh` is the X25519 shared secret `X25519(own_sk, other_pub)`, supplied by the
/// caller (the X25519 primitive is not in this module; it is the next, crate-based step).
pub fn contact_root(own_x_pub: &[u8; 32], other_x_pub: &[u8; 32], dh: &[u8; 32]) -> [u8; 32] {
    let (lo, hi) = if own_x_pub <= other_x_pub { (own_x_pub, other_x_pub) } else { (other_x_pub, own_x_pub) };
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(lo);
    salt[32..].copy_from_slice(hi);
    let mut out = [0u8; 32];
    hkdf(&salt, dh, CTX_CONTACT, &mut out);
    out
}

/// Recipient tag (spec 5.4): a 16-byte, direction-aware, hourly-rotating tag only the recipient can recognise.
pub fn recipient_tag(root: &[u8; 32], sender_x_pub: &[u8; 32], recipient_x_pub: &[u8; 32], period: u64) -> [u8; 16] {
    let dir: u8 = if sender_x_pub < recipient_x_pub { 0x00 } else { 0x01 };
    let mac = hmac_parts(root, &[CTX_TAG, &[dir], &period.to_be_bytes()]);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&mac[..16]);
    tag
}

/// The recipient-tag period for a unix time (spec 3: one hour).
pub fn tag_period(unix_seconds: u64) -> u64 {
    unix_seconds / 3600
}

/// Beacon tag (spec 5.5): an 8-byte tag rotating every 15 minutes; contacts recognise it, strangers see noise.
pub fn beacon_tag(beacon_key: &[u8; 32], unix_seconds: u64) -> [u8; 8] {
    let period = unix_seconds / 900;
    let mac = hmac_parts(beacon_key, &[CTX_BEACON, &period.to_be_bytes()]);
    let mut tag = [0u8; 8];
    tag.copy_from_slice(&mac[..8]);
    tag
}

/// Short authentication string (spec 4.3): six decimal digits from the Noise handshake hash `h`.
pub fn sas(handshake_hash: &[u8; 32]) -> u32 {
    let mut c = Sha256::new();
    c.update(CTX_SAS);
    c.update(handshake_hash);
    let d = c.finish();
    u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000
}

// ---- AFR-0009 wire layouts --------------------------------------------------------------------------

/// The message an identity record signs (spec 5.2): `CTX_IDENTITY || SHA-256(ed_pub || x_pub)`. The Ed25519
/// signature over this is produced later (signing is the crate step); this fixes the bytes that get signed.
pub fn identity_signed_message(ed_pub: &[u8; 32], x_pub: &[u8; 32]) -> [u8; 60] {
    let mut body = Sha256::new();
    body.update(ed_pub);
    body.update(x_pub);
    let h = body.finish();
    let mut out = [0u8; 60];
    out[..CTX_IDENTITY.len()].copy_from_slice(CTX_IDENTITY);
    out[CTX_IDENTITY.len()..].copy_from_slice(&h);
    out
}

/// Assemble the 128-byte identity record (spec 5.2): `ed_pub(32) || x_pub(32) || signature(64)`.
pub fn identity_record(ed_pub: &[u8; 32], x_pub: &[u8; 32], signature: &[u8; 64]) -> [u8; 128] {
    let mut rec = [0u8; 128];
    rec[0..32].copy_from_slice(ed_pub);
    rec[32..64].copy_from_slice(x_pub);
    rec[64..128].copy_from_slice(signature);
    rec
}

/// Assemble the 44-byte bundle header (spec 6). `bucket` is 0..=3.
pub fn bundle_header(ttl: u8, bucket: u8, bundle_id: &[u8; 16], tag: &[u8; 16], expires: u64) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0] = 1; // version
    h[1] = 0; // flags (reserved)
    h[2] = ttl;
    h[3] = bucket;
    h[4..20].copy_from_slice(bundle_id);
    h[20..36].copy_from_slice(tag);
    h[36..44].copy_from_slice(&expires.to_le_bytes());
    h
}

/// The AAD bound into the bundle seal (spec 6): the 44-byte header with the `ttl` byte set to 0, so a relay can
/// decrement `ttl` without breaking the seal while every other header field stays authenticated.
pub fn bundle_aad(header: &[u8; 44]) -> [u8; 44] {
    let mut aad = *header;
    aad[2] = 0;
    aad
}

// =====================================================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
    fn arr32(s: &str) -> [u8; 32] {
        let v = unhex(s);
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }

    // Public RFC 7748 section 6.1 constants (the X25519 primitive is not in this crate; these are its
    // published outputs, used so the derivations can be vector-tested without doing X25519 here).
    const PA: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
    const PB: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
    const DH: &str = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";
    const EDA: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const T: u64 = 1794225600;

    #[test]
    fn sha256_fips_abc() {
        assert_eq!(
            digest(b"abc").to_vec(),
            unhex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn hkdf_rfc5869_a1() {
        // RFC 5869 appendix A.1 test case 1.
        let salt = unhex("000102030405060708090a0b0c");
        let ikm = vec![0x0bu8; 22];
        let info = unhex("f0f1f2f3f4f5f6f7f8f9");
        let mut okm = [0u8; 42];
        hkdf(&salt, &ikm, &info, &mut okm);
        assert_eq!(
            okm.to_vec(),
            unhex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")
        );
    }

    #[test]
    fn contact_root_symmetric_and_matches_vector() {
        let (pa, pb, dh) = (arr32(PA), arr32(PB), arr32(DH));
        let root_ab = contact_root(&pa, &pb, &dh);
        let root_ba = contact_root(&pb, &pa, &dh);
        assert_eq!(root_ab, root_ba, "contact root must be identical from both sides");
        assert_eq!(
            root_ab.to_vec(),
            unhex("14af56d88b42b1fad7ad534fc8a312b8f8e4f2137694e6c3cfe85a3dc4844e00")
        );
    }

    #[test]
    fn recipient_tags_match_vector() {
        let (pa, pb, dh) = (arr32(PA), arr32(PB), arr32(DH));
        let root = contact_root(&pa, &pb, &dh);
        let period = tag_period(T);
        assert_eq!(period, 498396);
        assert_eq!(recipient_tag(&root, &pa, &pb, period).to_vec(), unhex("5a19f125776210e2961e5d3b64a82271"));
        assert_eq!(recipient_tag(&root, &pb, &pa, period).to_vec(), unhex("75c08454c83e4397069d1aed9fa2375c"));
        assert_eq!(recipient_tag(&root, &pa, &pb, period + 1).to_vec(), unhex("4415e70ef6962494f882d0a68923a35a"));
    }

    #[test]
    fn beacon_tag_matches_vector() {
        let key = [0x42u8; 32];
        assert_eq!(beacon_tag(&key, T).to_vec(), unhex("304197d51d6135f7"));
    }

    #[test]
    fn sas_matches_vector() {
        let h = {
            let mut c = Sha256::new();
            c.update(b"example Noise handshake hash");
            c.finish()
        };
        assert_eq!(sas(&h), 546787);
    }

    #[test]
    fn identity_signed_message_matches_vector() {
        let msg = identity_signed_message(&arr32(EDA), &arr32(PA));
        assert_eq!(
            msg.to_vec(),
            unhex("41465249534f564552412d4d4553482d4944454e544954592d563100422e8dd4b8aed6b9cf40567efd79e9a4d5fb85412857331a2a9764e54add39f4")
        );
    }

    #[test]
    fn bundle_header_and_aad_match_vector() {
        let (pa, pb, dh) = (arr32(PA), arr32(PB), arr32(DH));
        let root = contact_root(&pa, &pb, &dh);
        let tag = recipient_tag(&root, &pa, &pb, tag_period(T));
        let mut bid = [0u8; 16];
        for i in 0..16 {
            bid[i] = i as u8;
        }
        let hdr = bundle_header(8, 0, &bid, &tag, T + 86400);
        assert_eq!(
            hdr.to_vec(),
            unhex("01000800000102030405060708090a0b0c0d0e0f5a19f125776210e2961e5d3b64a822714007f36a00000000")
        );
        assert_eq!(
            bundle_aad(&hdr).to_vec(),
            unhex("01000000000102030405060708090a0b0c0d0e0f5a19f125776210e2961e5d3b64a822714007f36a00000000")
        );
    }
}
