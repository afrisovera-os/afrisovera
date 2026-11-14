//! A small no_std Noise driver for the mesh kernel port: patterns XX and KK, DH 25519, cipher ChaCha20-Poly1305,
//! hash SHA-256 (the AFR-0009 suite). Built only on primitives we already vet (x25519-dalek, chacha20poly1305,
//! mesh-crypto's SHA-256/HMAC), so it compiles for `aarch64-unknown-none`, unlike snow.
//!
//! Implements the Noise framework (revision 34) for these two patterns only. Ephemerals are supplied by the
//! caller (one per side), so the crate needs no RNG and is deterministic and testable; the kernel passes bytes
//! from the hardened DRBG. Correctness is proven by interop with snow (see tests): the handshake completes, the
//! handshake hashes agree, and transport messages round-trip, for both XX and KK and in both roles.

#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::vec::Vec;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use mesh_crypto::sha256::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

const NAME_XX: &[u8] = b"Noise_XX_25519_ChaChaPoly_SHA256"; // exactly 32 bytes
const NAME_KK: &[u8] = b"Noise_KK_25519_ChaChaPoly_SHA256"; // exactly 32 bytes

/// X25519 public key for a secret scalar (clamped by dalek, matching snow).
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

/// X25519, refusing an all-zero result (security finding M1, 13 Nov 2026). A low-order peer key (32 zero bytes,
/// for example) gives zero whatever our secret is, so a stranger with no private key could otherwise finish a
/// handshake whose keys are made of public bytes. RFC 7748 section 6.1 suggests this check and the Noise spec
/// allows it for 25519; mesh-hpke makes the same check (`was_contributory`, constant time).
fn dh(secret: &[u8; 32], peer: &[u8; 32]) -> Result<[u8; 32], ()> {
    let shared = StaticSecret::from(*secret).diffie_hellman(&PublicKey::from(*peer));
    if shared.was_contributory() {
        Ok(shared.to_bytes())
    } else {
        Err(())
    }
}

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut c = Sha256::new();
    for p in parts {
        c.update(p);
    }
    c.finish()
}

// Noise HKDF with two outputs: temp = HMAC(ck, ikm); out1 = HMAC(temp, 0x01); out2 = HMAC(temp, out1 || 0x02).
// F1: `temp` is key material; it is wiped before return (the outputs are the caller's to wipe).
fn hkdf2(ck: &[u8; 32], ikm: &[u8]) -> ([u8; 32], [u8; 32]) {
    let mut temp = mesh_crypto::hmac_parts(ck, &[ikm]);
    let o1 = mesh_crypto::hmac_parts(&temp, &[&[0x01u8]]);
    let o2 = mesh_crypto::hmac_parts(&temp, &[&o1, &[0x02u8]]);
    temp.zeroize();
    (o1, o2)
}

// Noise ChaChaPoly nonce: 32 bits of zeros followed by the 64-bit little-endian counter.
fn nonce12(n: u64) -> [u8; 12] {
    let mut x = [0u8; 12];
    x[4..].copy_from_slice(&n.to_le_bytes());
    x
}

// Review P6: never panic on a path a peer can reach - a kernel panic halts the machine. Returns None instead
// (the AEAD only refuses on inputs we never produce, but we handle it gracefully).
fn aead_seal(k: &[u8; 32], n: u64, ad: &[u8], pt: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(k))
        .encrypt(Nonce::from_slice(&nonce12(n)), Payload { msg: pt, aad: ad })
        .ok()
}

fn aead_open(k: &[u8; 32], n: u64, ad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(k))
        .decrypt(Nonce::from_slice(&nonce12(n)), Payload { msg: ct, aad: ad })
        .ok()
}

#[derive(Clone, Copy)]
enum Tok {
    E,
    S,
    Ee,
    Es,
    Se,
    Ss,
}

const XX_PAT: &[&[Tok]] = &[&[Tok::E], &[Tok::E, Tok::Ee, Tok::S, Tok::Es], &[Tok::S, Tok::Se]];
const KK_PAT: &[&[Tok]] = &[&[Tok::E, Tok::Es, Tok::Ss], &[Tok::E, Tok::Ee, Tok::Se]];

/// A Noise handshake in progress. Drive it with alternating `write_message` / `read_message`.
pub struct Handshake {
    ck: [u8; 32],
    h: [u8; 32],
    k: Option<[u8; 32]>,
    n: u64,
    s_priv: [u8; 32],
    s_pub: [u8; 32],
    e_priv: [u8; 32],
    e_pub: [u8; 32],
    rs: Option<[u8; 32]>,
    re: Option<[u8; 32]>,
    initiator: bool,
    patterns: &'static [&'static [Tok]],
    idx: usize,
    failed: bool,
}

impl Handshake {
    fn new(name: &[u8], prologue: &[u8], s_priv: [u8; 32], e_priv: [u8; 32], initiator: bool, patterns: &'static [&'static [Tok]], premsg: &[[u8; 32]]) -> Self {
        // InitializeSymmetric: both suite names are exactly 32 bytes, so h = name (no hash); ck = h.
        let mut h = [0u8; 32];
        if name.len() <= 32 {
            h[..name.len()].copy_from_slice(name);
        } else {
            h = sha(&[name]);
        }
        let ck = h;
        let mut hs = Self {
            ck,
            h,
            k: None,
            n: 0,
            s_priv,
            s_pub: public_key(&s_priv),
            e_priv,
            e_pub: public_key(&e_priv),
            rs: None,
            re: None,
            initiator,
            patterns,
            idx: 0,
            failed: false,
        };
        hs.mixhash(prologue);
        for pk in premsg {
            hs.mixhash(pk);
        }
        hs
    }

    /// XX initiator (first meeting / pairing).
    pub fn xx_initiator(s_priv: [u8; 32], e_priv: [u8; 32], prologue: &[u8]) -> Self {
        Self::new(NAME_XX, prologue, s_priv, e_priv, true, XX_PAT, &[])
    }
    /// XX responder.
    pub fn xx_responder(s_priv: [u8; 32], e_priv: [u8; 32], prologue: &[u8]) -> Self {
        Self::new(NAME_XX, prologue, s_priv, e_priv, false, XX_PAT, &[])
    }
    /// KK initiator dialing the contact whose static key is `rs`.
    pub fn kk_initiator(s_priv: [u8; 32], e_priv: [u8; 32], rs: [u8; 32], prologue: &[u8]) -> Self {
        let own = public_key(&s_priv);
        let mut hs = Self::new(NAME_KK, prologue, s_priv, e_priv, true, KK_PAT, &[own, rs]);
        hs.rs = Some(rs);
        hs
    }
    /// KK responder expecting the contact whose static key is `rs`.
    pub fn kk_responder(s_priv: [u8; 32], e_priv: [u8; 32], rs: [u8; 32], prologue: &[u8]) -> Self {
        let own = public_key(&s_priv);
        let mut hs = Self::new(NAME_KK, prologue, s_priv, e_priv, false, KK_PAT, &[rs, own]);
        hs.rs = Some(rs);
        hs
    }

    fn mixhash(&mut self, data: &[u8]) {
        self.h = sha(&[&self.h, data]);
    }

    fn mixkey(&mut self, ikm: &[u8; 32]) {
        let (mut ck, mut k) = hkdf2(&self.ck, ikm);
        self.ck = ck;
        if let Some(old) = self.k.as_mut() {
            old.zeroize();
        }
        self.k = Some(k);
        self.n = 0;
        ck.zeroize();
        k.zeroize();
    }

    fn encrypt_and_hash(&mut self, pt: &[u8]) -> Option<Vec<u8>> {
        let out = match self.k.as_ref() {
            Some(k) => {
                let c = aead_seal(k, self.n, &self.h, pt)?;
                self.n += 1;
                c
            }
            None => pt.to_vec(),
        };
        self.mixhash(&out);
        Some(out)
    }

    fn decrypt_and_hash(&mut self, ct: &[u8]) -> Option<Vec<u8>> {
        let pt = match self.k.as_ref() {
            Some(k) => {
                let p = aead_open(k, self.n, &self.h, ct)?;
                self.n += 1;
                p
            }
            None => ct.to_vec(),
        };
        self.mixhash(ct);
        Some(pt)
    }

    fn mix_token(&mut self, t: Tok) -> Result<(), ()> {
        let mut d = match t {
            Tok::Ee => dh(&self.e_priv, &self.re.ok_or(())?)?,
            Tok::Es => {
                if self.initiator {
                    dh(&self.e_priv, &self.rs.ok_or(())?)?
                } else {
                    dh(&self.s_priv, &self.re.ok_or(())?)?
                }
            }
            Tok::Se => {
                if self.initiator {
                    dh(&self.s_priv, &self.re.ok_or(())?)?
                } else {
                    dh(&self.e_priv, &self.rs.ok_or(())?)?
                }
            }
            Tok::Ss => dh(&self.s_priv, &self.rs.ok_or(())?)?,
            _ => return Err(()),
        };
        self.mixkey(&d);
        d.zeroize();
        Ok(())
    }

    /// Write the next handshake message, appending it to `out`. On any error the handshake is poisoned
    /// (review P3): every later call returns `Err` rather than proceeding from a half-updated state.
    pub fn write_message(&mut self, payload: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        if self.failed {
            return Err(());
        }
        let start = out.len();
        let r = self.try_write(payload, out);
        if r.is_err() {
            self.failed = true;
            // nothing of a failed message leaves (M1: the e already appended before a refused DH)
            out.truncate(start);
        }
        r
    }

    fn try_write(&mut self, payload: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        if self.idx >= self.patterns.len() {
            return Err(());
        }
        for &t in self.patterns[self.idx] {
            match t {
                Tok::E => {
                    let ep = self.e_pub;
                    out.extend_from_slice(&ep);
                    self.mixhash(&ep);
                }
                Tok::S => {
                    let sp = self.s_pub;
                    let c = self.encrypt_and_hash(&sp).ok_or(())?;
                    out.extend_from_slice(&c);
                }
                other => self.mix_token(other)?,
            }
        }
        let c = self.encrypt_and_hash(payload).ok_or(())?;
        out.extend_from_slice(&c);
        self.idx += 1;
        Ok(())
    }

    /// Read the next handshake message, writing any payload into `out`. Poisons on any error (review P3).
    pub fn read_message(&mut self, msg: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        if self.failed {
            return Err(());
        }
        let r = self.try_read(msg, out);
        if r.is_err() {
            self.failed = true;
        }
        r
    }

    fn try_read(&mut self, msg: &[u8], out: &mut Vec<u8>) -> Result<(), ()> {
        if self.idx >= self.patterns.len() {
            return Err(());
        }
        let mut cur = msg;
        for &t in self.patterns[self.idx] {
            match t {
                Tok::E => {
                    if cur.len() < 32 {
                        return Err(());
                    }
                    let mut re = [0u8; 32];
                    re.copy_from_slice(&cur[..32]);
                    self.re = Some(re);
                    self.mixhash(&re);
                    cur = &cur[32..];
                }
                Tok::S => {
                    let len = if self.k.is_some() { 48 } else { 32 };
                    if cur.len() < len {
                        return Err(());
                    }
                    let pt = self.decrypt_and_hash(&cur[..len]).ok_or(())?;
                    if pt.len() != 32 {
                        return Err(());
                    }
                    let mut rs = [0u8; 32];
                    rs.copy_from_slice(&pt);
                    self.rs = Some(rs);
                    cur = &cur[len..];
                }
                other => self.mix_token(other)?,
            }
        }
        let pt = self.decrypt_and_hash(cur).ok_or(())?;
        out.extend_from_slice(&pt);
        self.idx += 1;
        Ok(())
    }

    /// True once all handshake messages have been processed.
    pub fn is_finished(&self) -> bool {
        self.idx >= self.patterns.len()
    }

    /// The Noise handshake hash (feeds the AFR-0009 SAS). Call once finished.
    pub fn handshake_hash(&self) -> [u8; 32] {
        self.h
    }

    /// The remote static key learned during the handshake (XX), if any.
    pub fn remote_static(&self) -> Option<[u8; 32]> {
        self.rs
    }

    /// Split into the two transport cipher states: (send, receive) for this side's role.
    pub fn split(self) -> (CipherState, CipherState) {
        let (mut k1, mut k2) = hkdf2(&self.ck, &[]);
        let pair = if self.initiator {
            (CipherState::new(k1), CipherState::new(k2))
        } else {
            (CipherState::new(k2), CipherState::new(k1))
        };
        k1.zeroize();
        k2.zeroize();
        pair
        // `self` drops here, which wipes ck, k and both private keys (F1).
    }
}

/// F1 (review S2): a handshake wipes its chaining key, cipher key and both private keys when it is dropped,
/// whether it finished, failed or was abandoned. The transcript hash `h` is not secret (it feeds the SAS).
impl Drop for Handshake {
    fn drop(&mut self) {
        self.ck.zeroize();
        if let Some(k) = self.k.as_mut() {
            k.zeroize();
        }
        self.k = None;
        self.s_priv.zeroize();
        self.e_priv.zeroize();
    }
}

/// A post-handshake transport cipher state (one direction).
pub struct CipherState {
    k: [u8; 32],
    n: u64,
}

impl CipherState {
    fn new(k: [u8; 32]) -> Self {
        Self { k, n: 0 }
    }
    /// Encrypt a transport message with associated data `ad`. `None` once the nonce space is exhausted (the
    /// reserved value 2^64-1 is never used for data - review P5) or if the AEAD refuses.
    pub fn encrypt(&mut self, ad: &[u8], pt: &[u8]) -> Option<Vec<u8>> {
        if self.n == u64::MAX {
            return None;
        }
        let c = aead_seal(&self.k, self.n, ad, pt)?;
        self.n += 1;
        Some(c)
    }
    /// Decrypt a transport message; `None` on authentication failure or an exhausted nonce (review P5).
    pub fn decrypt(&mut self, ad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
        if self.n == u64::MAX {
            return None;
        }
        let p = aead_open(&self.k, self.n, ad, ct)?;
        self.n += 1;
        Some(p)
    }
    /// Noise Rekey (review P5): derive a fresh key without advancing the nonce. AFR-0009 section 4.2 rekeys
    /// after 10000 messages or 1 hour; call this before the nonce nears exhaustion.
    pub fn rekey(&mut self) {
        if let Some(mut out) = aead_seal(&self.k, u64::MAX, &[], &[0u8; 32]) {
            self.k.copy_from_slice(&out[..32]);
            out.zeroize();
        }
    }
}

/// F1 (review S2): a transport key is wiped when its link is dropped.
impl Drop for CipherState {
    fn drop(&mut self) {
        self.k.zeroize();
    }
}

// =====================================================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn arr32(s: &str) -> [u8; 32] {
        let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
    const XA: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
    const XB: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";

    fn pro() -> Vec<u8> {
        let mut p = b"AFRISOVERA-MESH-LINK-V1\0".to_vec();
        p.push(0x03);
        p
    }

    // Transport round-trip between our CipherStates and a snow TransportState (ours sends, snow receives, etc.).
    fn exchange(mine_send: &mut CipherState, mine_recv: &mut CipherState, snow_t: &mut snow::TransportState) {
        let ct = mine_send.encrypt(&[], b"habari").unwrap();
        let mut out = [0u8; 256];
        let n = snow_t.read_message(&ct, &mut out).unwrap();
        assert_eq!(&out[..n], b"habari");
        let mut sbuf = [0u8; 256];
        let n = snow_t.write_message(b"salama", &mut sbuf).unwrap();
        let pt = mine_recv.decrypt(&[], &sbuf[..n]).unwrap();
        assert_eq!(pt, b"salama");
    }

    #[test]
    fn xx_interop_with_snow_mine_initiator() {
        let a = arr32(XA);
        let b = arr32(XB);
        let p = pro();
        let mut mine = Handshake::xx_initiator(a, [0x11u8; 32], &p);
        let mut theirs = snow::Builder::new("Noise_XX_25519_ChaChaPoly_SHA256".parse().unwrap())
            .prologue(&p)
            .local_private_key(&b)
            .build_responder()
            .unwrap();
        let mut tmp = [0u8; 512];
        let mut m1 = Vec::new();
        mine.write_message(&[], &mut m1).unwrap();
        theirs.read_message(&m1, &mut tmp).unwrap();
        let mut sbuf = [0u8; 512];
        let n = theirs.write_message(&[], &mut sbuf).unwrap();
        let mut pl = Vec::new();
        mine.read_message(&sbuf[..n], &mut pl).unwrap();
        let mut m3 = Vec::new();
        mine.write_message(&[], &mut m3).unwrap();
        theirs.read_message(&m3, &mut tmp).unwrap();
        assert!(mine.is_finished());
        assert_eq!(mine.handshake_hash().to_vec(), theirs.get_handshake_hash().to_vec(), "handshake hashes agree with snow");
        assert_eq!(mine.remote_static(), Some(public_key(&b)), "XX learns the peer static");
        let (mut s, mut r) = mine.split();
        let mut tt = theirs.into_transport_mode().unwrap();
        exchange(&mut s, &mut r, &mut tt);
    }

    #[test]
    fn xx_interop_with_snow_mine_responder() {
        let a = arr32(XA);
        let b = arr32(XB);
        let p = pro();
        let mut theirs = snow::Builder::new("Noise_XX_25519_ChaChaPoly_SHA256".parse().unwrap())
            .prologue(&p)
            .local_private_key(&a)
            .build_initiator()
            .unwrap();
        let mut mine = Handshake::xx_responder(b, [0x22u8; 32], &p);
        let mut tmp = [0u8; 512];
        let mut sbuf = [0u8; 512];
        let n = theirs.write_message(&[], &mut sbuf).unwrap();
        let mut pl = Vec::new();
        mine.read_message(&sbuf[..n], &mut pl).unwrap();
        let mut m2 = Vec::new();
        mine.write_message(&[], &mut m2).unwrap();
        theirs.read_message(&m2, &mut tmp).unwrap();
        let n = theirs.write_message(&[], &mut sbuf).unwrap();
        let mut pl2 = Vec::new();
        mine.read_message(&sbuf[..n], &mut pl2).unwrap();
        assert!(mine.is_finished());
        assert_eq!(mine.handshake_hash().to_vec(), theirs.get_handshake_hash().to_vec());
        // snow is the initiator here; its send cipher pairs with our receive cipher.
        let (mut s, mut r) = mine.split();
        let mut tt = theirs.into_transport_mode().unwrap();
        // snow sends first here: snow -> mine.
        let n = tt.write_message(b"habari", &mut sbuf).unwrap();
        assert_eq!(r.decrypt(&[], &sbuf[..n]).unwrap(), b"habari");
        let ct = s.encrypt(&[], b"salama").unwrap();
        let mut out = [0u8; 256];
        let n = tt.read_message(&ct, &mut out).unwrap();
        assert_eq!(&out[..n], b"salama");
    }

    #[test]
    fn kk_interop_with_snow() {
        let a = arr32(XA);
        let b = arr32(XB);
        let pa = public_key(&a);
        let pb = public_key(&b);
        let p = pro();
        let mut mine = Handshake::kk_initiator(a, [0x33u8; 32], pb, &p);
        let mut theirs = snow::Builder::new("Noise_KK_25519_ChaChaPoly_SHA256".parse().unwrap())
            .prologue(&p)
            .local_private_key(&b)
            .remote_public_key(&pa)
            .build_responder()
            .unwrap();
        let mut tmp = [0u8; 512];
        let mut sbuf = [0u8; 512];
        let mut m1 = Vec::new();
        mine.write_message(&[], &mut m1).unwrap();
        theirs.read_message(&m1, &mut tmp).unwrap();
        let n = theirs.write_message(&[], &mut sbuf).unwrap();
        let mut pl = Vec::new();
        mine.read_message(&sbuf[..n], &mut pl).unwrap();
        assert!(mine.is_finished());
        assert_eq!(mine.handshake_hash().to_vec(), theirs.get_handshake_hash().to_vec());
        let (mut s, mut r) = mine.split();
        let mut tt = theirs.into_transport_mode().unwrap();
        exchange(&mut s, &mut r, &mut tt);
    }

    #[test]
    fn self_consistent_xx() {
        // Our initiator with our responder (no snow): proves internal consistency and transport both ways.
        let a = arr32(XA);
        let b = arr32(XB);
        let p = pro();
        let mut i = Handshake::xx_initiator(a, [0x44u8; 32], &p);
        let mut r = Handshake::xx_responder(b, [0x55u8; 32], &p);
        let (mut m1, mut m2, mut m3, mut pl) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        i.write_message(&[], &mut m1).unwrap();
        r.read_message(&m1, &mut pl).unwrap();
        r.write_message(&[], &mut m2).unwrap();
        i.read_message(&m2, &mut pl).unwrap();
        i.write_message(&[], &mut m3).unwrap();
        r.read_message(&m3, &mut pl).unwrap();
        assert_eq!(i.handshake_hash(), r.handshake_hash());
        let (mut is, mut ir) = i.split();
        let (mut rs, mut rr) = r.split();
        let c = is.encrypt(b"", b"ping").unwrap();
        assert_eq!(rr.decrypt(b"", &c).unwrap(), b"ping");
        let c = rs.encrypt(b"", b"pong").unwrap();
        assert_eq!(ir.decrypt(b"", &c).unwrap(), b"pong");
    }

    // ---- Review P4: failure cases ----------------------------------------------------------------

    #[test]
    fn tampered_handshake_message_is_rejected_and_poisons() {
        let a = arr32(XA);
        let b = arr32(XB);
        let p = pro();
        let mut i = Handshake::xx_initiator(a, [0x11u8; 32], &p);
        let mut r = Handshake::xx_responder(b, [0x22u8; 32], &p);
        let mut m1 = Vec::new();
        i.write_message(&[], &mut m1).unwrap();
        m1[5] ^= 0x01; // flip a byte of the initiator's ephemeral
        let mut pl = Vec::new();
        // The handshake still completes the DH, but the mismatch surfaces when the responder decrypts a later
        // payload; regardless, a tampered run must never yield a working transport. Here we corrupt the FINAL
        // message instead to get a deterministic decrypt failure:
        let _ = r.read_message(&m1, &mut pl); // reads e (no auth yet); may or may not error
        // Force a clean deterministic case: fresh pair, tamper the authenticated msg2.
        let mut i2 = Handshake::xx_initiator(a, [0x11u8; 32], &p);
        let mut r2 = Handshake::xx_responder(b, [0x22u8; 32], &p);
        let (mut m1b, mut m2b, mut plb) = (Vec::new(), Vec::new(), Vec::new());
        i2.write_message(&[], &mut m1b).unwrap();
        r2.read_message(&m1b, &mut plb).unwrap();
        r2.write_message(&[], &mut m2b).unwrap();
        m2b[40] ^= 0x01; // corrupt an AEAD-protected byte of msg2
        assert!(i2.read_message(&m2b, &mut plb).is_err(), "tampered msg2 rejected");
        // Poisoned: any further call errors (review P3).
        assert!(i2.read_message(&m2b, &mut plb).is_err(), "poisoned after failure");
        let mut junk = Vec::new();
        assert!(i2.write_message(&[], &mut junk).is_err(), "poisoned for writes too");
    }

    #[test]
    fn truncated_message_is_rejected() {
        let b = arr32(XB);
        let p = pro();
        let mut r = Handshake::xx_responder(b, [0x22u8; 32], &p);
        let mut pl = Vec::new();
        assert!(r.read_message(&[0u8; 10], &mut pl).is_err(), "a 10-byte msg1 (needs >=32 for e) is rejected");
    }

    #[test]
    fn use_after_finish_is_rejected() {
        let a = arr32(XA);
        let b = arr32(XB);
        let p = pro();
        let mut i = Handshake::xx_initiator(a, [0x44u8; 32], &p);
        let mut r = Handshake::xx_responder(b, [0x55u8; 32], &p);
        let (mut m1, mut m2, mut m3, mut pl) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        i.write_message(&[], &mut m1).unwrap();
        r.read_message(&m1, &mut pl).unwrap();
        r.write_message(&[], &mut m2).unwrap();
        i.read_message(&m2, &mut pl).unwrap();
        i.write_message(&[], &mut m3).unwrap();
        assert!(i.is_finished());
        let mut extra = Vec::new();
        assert!(i.write_message(&[], &mut extra).is_err(), "write after finish is rejected");
    }

    #[test]
    fn kk_wrong_peer_static_is_rejected() {
        let a = arr32(XA);
        let b = arr32(XB);
        let pa = public_key(&a);
        let p = pro();
        // Initiator dials with the WRONG responder static (A's own key, not B's).
        let mut init_bad = Handshake::kk_initiator(a, [0x66u8; 32], pa, &p);
        let mut resp = Handshake::kk_responder(b, [0x77u8; 32], pa, &p);
        let mut m1 = Vec::new();
        init_bad.write_message(&[], &mut m1).unwrap();
        let mut pl = Vec::new();
        assert!(resp.read_message(&m1, &mut pl).is_err(), "KK with a wrong peer static is rejected");
        assert!(resp.read_message(&m1, &mut pl).is_err(), "and the responder is poisoned");
    }

    #[test]
    fn rekey_changes_the_key() {
        let k = [0x01u8; 32];
        let mut c1 = CipherState::new(k);
        let mut c2 = CipherState::new(k);
        c2.rekey();
        // Same nonce counter (0) but different keys now, so ciphertexts differ.
        assert_ne!(c1.encrypt(b"", b"x").unwrap(), c2.encrypt(b"", b"x").unwrap());
    }

    // F1: dropping a handshake wipes its private keys, chaining key and cipher key; dropping a transport
    // state wipes its key. Checked through ManuallyDrop so the fields can be read after the destructor ran.
    #[test]
    fn drop_wipes_secrets() {
        let a = arr32(XA);
        let b = arr32(XB);
        let mut i = Handshake::xx_initiator(a, b, &pro());
        let mut r = Handshake::xx_responder(b, a, &pro());
        let (mut m, mut pl) = (Vec::new(), Vec::new());
        i.write_message(&[], &mut m).unwrap();
        r.read_message(&m, &mut pl).unwrap();
        m.clear();
        r.write_message(&[], &mut m).unwrap();
        i.read_message(&m, &mut pl).unwrap();
        assert!(i.k.is_some() && i.ck != [0u8; 32]);
        let mut hs = core::mem::ManuallyDrop::new(i);
        // SAFETY: dropped once, then only plain byte arrays are read; ManuallyDrop never drops it again.
        unsafe { core::ptr::drop_in_place(&mut *hs) };
        assert_eq!(hs.s_priv, [0u8; 32]);
        assert_eq!(hs.e_priv, [0u8; 32]);
        assert_eq!(hs.ck, [0u8; 32]);
        assert!(hs.k.is_none());

        let mut cs = core::mem::ManuallyDrop::new(CipherState::new([7u8; 32]));
        // SAFETY: as above.
        unsafe { core::ptr::drop_in_place(&mut *cs) };
        assert_eq!(cs.k, [0u8; 32]);
    }

    // Security finding M1 (13 Nov 2026): a stranger with no private key sends 32 zero bytes as its ephemeral.
    // XX m1 is only "e", so reading it does no DH; the responder's ee (and es) come when it writes m2. That
    // write must fail, leave no m2 bytes (so the responder's static key is never sent) and poison the handshake.
    #[test]
    fn m1_zero_ephemeral_is_refused_by_the_responder() {
        let b = arr32(XB);
        let mut r = Handshake::xx_responder(b, [0x55u8; 32], &pro());
        let m1 = [0u8; 32];
        let mut pl = Vec::new();
        r.read_message(&m1, &mut pl).unwrap();
        let mut m2 = Vec::new();
        assert!(r.write_message(&[], &mut m2).is_err(), "the responder answers a zero ephemeral");
        assert!(m2.is_empty(), "m2 bytes left the responder");
        assert!(r.write_message(&[], &mut m2).is_err(), "not poisoned");
        assert!(r.read_message(&m1, &mut pl).is_err(), "not poisoned for reads");
    }

    // M1, the other low-order points too: each of the seven low-order X25519 u-coordinates in RFC 7748's
    // neighbourhood (libsodium's list), with and without the ignored top bit, is refused when the responder
    // answers it.
    #[test]
    fn m1_every_low_order_ephemeral_is_refused() {
        let b = arr32(XB);
        let low_order = [
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
            "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
            "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        ];
        for (n, h) in low_order.iter().enumerate() {
            for top in [0u8, 0x80] {
                let mut e = arr32(h);
                e[31] |= top;
                let mut r = Handshake::xx_responder(b, [0x55u8; 32], &pro());
                let (mut pl, mut m2) = (Vec::new(), Vec::new());
                r.read_message(&e, &mut pl).unwrap();
                assert!(r.write_message(&[], &mut m2).is_err(), "low-order point {} (top bit {:#x}) accepted", n, top);
                assert!(m2.is_empty());
            }
        }
    }

    // M1 from the initiator's side (`mesh connect` against a fake responder): an m2 whose ephemeral is zero is
    // refused before anything in it is decrypted, and the initiator writes no m3.
    #[test]
    fn m1_zero_ephemeral_in_m2_is_refused_by_the_initiator() {
        let a = arr32(XA);
        let mut i = Handshake::xx_initiator(a, [0x44u8; 32], &pro());
        let mut m1 = Vec::new();
        i.write_message(&[], &mut m1).unwrap();
        let mut m2 = vec![0u8; 32];
        m2.extend_from_slice(&[0u8; 48 + 16]);
        let mut pl = Vec::new();
        assert!(i.read_message(&m2, &mut pl).is_err(), "m2 with a zero ephemeral is read");
        assert!(pl.is_empty());
        let mut m3 = Vec::new();
        assert!(i.write_message(&[], &mut m3).is_err(), "the initiator writes m3 after a zero ephemeral");
    }

    // M1, the static half: a real ephemeral but a zero static in m3 makes se zero for the responder. The
    // stranger cannot normally encrypt m3 (it needs ee and es), so this drives a real initiator whose static key
    // is replaced after m2 by one whose public half is zero: se = DH(e_r, 0) must be refused.
    #[test]
    fn m1_zero_static_in_m3_is_refused_by_the_responder() {
        let a = arr32(XA);
        let b = arr32(XB);
        let mut i = Handshake::xx_initiator(a, [0x44u8; 32], &pro());
        let mut r = Handshake::xx_responder(b, [0x55u8; 32], &pro());
        let (mut m1, mut m2, mut m3, mut pl) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        i.write_message(&[], &mut m1).unwrap();
        r.read_message(&m1, &mut pl).unwrap();
        r.write_message(&[], &mut m2).unwrap();
        i.read_message(&m2, &mut pl).unwrap();
        i.s_pub = [0u8; 32];
        // The initiator's own se uses its real secret, so it writes m3; the responder's se = DH(e_r, 0) = 0.
        i.write_message(&[], &mut m3).unwrap();
        assert!(r.read_message(&m3, &mut pl).is_err(), "m3 with a zero static is accepted");
        assert!(!r.is_finished());
    }

    // KK with a zero peer static configured: ss and the static DH are zero, so the first message is refused.
    #[test]
    fn m1_kk_zero_peer_static_is_refused() {
        let a = arr32(XA);
        let mut init = Handshake::kk_initiator(a, [0x66u8; 32], [0u8; 32], &pro());
        let mut m1 = Vec::new();
        assert!(init.write_message(&[], &mut m1).is_err(), "KK initiator writes to a zero static");
    }
}
