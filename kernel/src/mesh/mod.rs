//! Sovereign Mesh, Phase 0, in the kernel (AFR-0009; plan of record docs/design/mesh-engineering-plan.md;
//! port plan docs/design/mesh-phase0-port-plan.md; ADR 0007 for the crates).
//!
//! What Phase 0 shows: two AfriSovera machines, no internet, run the AFR-0009 Layer B1 link (Noise XX over TCP,
//! transport byte 0x03) and pass ONE sealed AFR-0009 bundle (Layer B2, HPKE mode_auth) that the recipient opens.
//!
//! The pieces are the host-validated crates, unchanged: `mesh-crypto` (derivations, tags, SAS, wire layout),
//! `mesh-noise` (no_std Noise XX/KK, snow-interop verified, poisons on any error) and `mesh-hpke` (X25519,
//! ChaCha20-Poly1305, HPKE mode_auth, bundle seal/open with the P1/P2 checks). This module adds only the kernel
//! glue: keys from the hardened DRBG (`rng::fill_secret`, fail closed), the stream framing, and the two TCP
//! roles through the netstack's handler hook (`virtio::net_tcp_serve_with` / `net_tcp_dial_with`).
//!
//! It meets the rest of the kernel ONLY through that hook and the DRBG (architect decision, 9 Nov, option A),
//! written to the AFR-0002 class-protocol shape so it can move to an EL0 process once channels exist.
//!
//! Phase 0 limits, stated plainly: bucket 0 (256 bytes) only, because the serve loop sends one segment of at
//! most 512 bytes per exchange (architect review, "size limit"); one link at a time; the identity key lives in
//! RAM for this boot only (the keystore, review R2-6, is a prerequisite for anything longer); the bundle's
//! `expires` is checked against its own `created`, not against a trusted clock.
//!
//! Secrets (F1, review S2): the identity key, ephemerals and every DH output, contact root and HPKE key are
//! wiped when they go out of use (`Secret` here, `Drop` on mesh-noise's handshake and transport states, the
//! dalek `zeroize` feature, and mesh-hpke's own wipes). `mesh forget` drops this boot's identity, which wipes it.
//! Peer text (F4) reaches the console only through `mesh_crypto::console_safe`.

use alloc::vec::Vec;
use core::ptr::addr_of_mut;

use mesh_noise::{CipherState, Handshake};

/// AFR-0009 section 8: the TCP binding's port.
pub const PORT: u16 = 4401;
/// AFR-0009 section 4.1 transport byte for TCP.
const TRANSPORT_TCP: u8 = 0x03;
/// AFR-0009 section 3 link prologue context.
const CTX_LINK: &[u8] = b"AFRISOVERA-MESH-LINK-V1\0";
/// Record types (AFR-0009 section 4.2).
const REC_BUNDLE: u8 = 0x01;
const REC_CLOSE: u8 = 0x06;
const REC_PING: u8 = 0x07;
/// The largest framed message Phase 0 accepts (a bucket-0 BUNDLE record is 2 + 1 + 256 + 16 = 275 bytes).
const MAX_FRAME: usize = 512;
/// Bucket 0 only in Phase 0 (see the module comment).
const BUCKET: usize = 0;
const TTL: u8 = 8;

/// A 32-byte secret that wipes itself when dropped (F1). Moves are bitwise copies the compiler may leave
/// behind; keeping secrets in this type wipes every named copy we own.
pub struct Secret(pub [u8; 32]);

impl Secret {
    fn copy(&self) -> Secret {
        Secret(self.0)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        crate::drbg::wipe(&mut self.0);
    }
}

/// This node's X25519 identity for this boot, generated on first use from `fill_secret`. Review M1: the secret
/// lives in ONE fixed buffer that is never moved or overwritten by an enum store; `forget()` wipes that buffer
/// where it lies and clears the flag. (An `Option<(Secret, _)>` here failed the M1 selftest even when set to
/// `None` in place: the store of `None` is not guaranteed to leave the payload bytes as the drop left them.)
static mut IDENTITY_SK: Secret = Secret([0u8; 32]);
static mut IDENTITY_PK: [u8; 32] = [0u8; 32];
static mut HAVE_IDENTITY: bool = false;

/// Why a mesh step failed, in words for the console and the suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshError {
    /// The DRBG refused a secret (no healthy strong source, or the KAT failed). Nothing was sent.
    NoSecretRandom,
    /// A fresh secret came back all zero (review S4's assertion).
    ZeroSecret,
    Handshake,
    Frame,
    Decrypt,
    Seal,
    Open,
    Tag,
    Unexpected,
}

/// 32 secret bytes from the hardened DRBG, or an error. Never all-zero (review S4).
fn secret32() -> Result<Secret, MeshError> {
    let mut k = Secret([0u8; 32]);
    crate::rng::fill_secret(&mut k.0).map_err(|_| MeshError::NoSecretRandom)?;
    if k.0.iter().all(|&b| b == 0) {
        return Err(MeshError::ZeroSecret);
    }
    Ok(k)
}

/// This node's (secret, public) X25519 identity, created on first use. The caller's copy wipes on drop.
pub fn identity() -> Result<(Secret, [u8; 32]), MeshError> {
    // SAFETY: single accessor (the shell core), like the rest of the kernel's statics.
    unsafe {
        let sk = &mut *addr_of_mut!(IDENTITY_SK);
        let pk = &mut *addr_of_mut!(IDENTITY_PK);
        if *addr_of_mut!(HAVE_IDENTITY) {
            return Ok((sk.copy(), *pk));
        }
        let fresh = secret32()?; // wiped when it drops at the end of this block
        sk.0.copy_from_slice(&fresh.0);
        *pk = mesh_noise::public_key(&sk.0);
        *addr_of_mut!(HAVE_IDENTITY) = true;
        Ok((sk.copy(), *pk))
    }
}

/// `mesh forget`: wipe this boot's identity where it lies (review M1) and forget it. The next mesh step makes a
/// new one. Returns false if there was none.
pub fn forget() -> bool {
    // SAFETY: single accessor (the shell core).
    unsafe {
        let had = *addr_of_mut!(HAVE_IDENTITY);
        crate::drbg::wipe(&mut (*addr_of_mut!(IDENTITY_SK)).0);
        crate::drbg::wipe(&mut *addr_of_mut!(IDENTITY_PK));
        *addr_of_mut!(HAVE_IDENTITY) = false;
        had
    }
}

/// Address of the identity secret's bytes inside the static (for the M1 selftest).
fn identity_secret_ptr() -> Option<*const u8> {
    // SAFETY: only an address is taken.
    Some(unsafe { (*addr_of_mut!(IDENTITY_SK)).0.as_ptr() })
}

fn prologue() -> [u8; 25] {
    let mut p = [0u8; 25];
    p[..24].copy_from_slice(CTX_LINK);
    p[24] = TRANSPORT_TCP;
    p
}

/// Wall-clock seconds (PL031 on virt; 0 where the board has no RTC we read).
fn now_unix() -> u64 {
    #[cfg(not(feature = "board-raspi"))]
    // SAFETY: the board's resolved RTC base is a mapped, readable MMIO register.
    let t = unsafe { core::ptr::read_volatile(crate::board::resolved().rtc as *const u32) } as u64;
    #[cfg(feature = "board-raspi")]
    let t = 0u64;
    t
}

fn hex8(k: &[u8]) -> [u8; 16] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut o = [b'0'; 16];
    for i in 0..8.min(k.len()) {
        o[i * 2] = HEX[(k[i] >> 4) as usize];
        o[i * 2 + 1] = HEX[(k[i] & 15) as usize];
    }
    o
}

fn s(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap_or("?")
}

/// F4: peer-supplied text as something safe to print between quotes (control bytes, bad UTF-8 and
/// invisible reordering characters escaped; see `mesh_crypto::console_safe`).
fn shown(text: &[u8]) -> alloc::string::String {
    let mut o = alloc::vec![0u8; mesh_crypto::console_safe_room(text.len())];
    let n = mesh_crypto::console_safe(text, &mut o);
    o.truncate(n);
    alloc::string::String::from_utf8(o).unwrap_or_default()
}

/// AFR-0009 section 4.2 stream framing: u16 LE length, then the Noise message.
fn push_frame(out: &mut Vec<u8>, msg: &[u8]) {
    out.extend_from_slice(&(msg.len() as u16).to_le_bytes());
    out.extend_from_slice(msg);
}

/// Reassembles length-prefixed frames across TCP segments (the netstack never parses Noise).
struct Framer {
    buf: Vec<u8>,
}

impl Framer {
    const fn new() -> Self {
        Framer { buf: Vec::new() }
    }
    fn push(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// The next whole frame, `Ok(None)` if more bytes are needed, `Err` on an over-long length.
    fn next(&mut self) -> Result<Option<Vec<u8>>, MeshError> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        let n = u16::from_le_bytes([self.buf[0], self.buf[1]]) as usize;
        if n == 0 || n > MAX_FRAME {
            return Err(MeshError::Frame);
        }
        if self.buf.len() < 2 + n {
            return Ok(None);
        }
        let f = self.buf[2..2 + n].to_vec();
        self.buf.drain(..2 + n);
        Ok(Some(f))
    }
}

/// An established link (both transport directions plus what the handshake taught us).
struct Link {
    tx: CipherState,
    rx: CipherState,
    peer: [u8; 32],
}

/// Encrypt `[type] || body` as one record and frame it.
fn push_record(out: &mut Vec<u8>, link: &mut Link, rtype: u8, body: &[u8]) -> Result<(), MeshError> {
    let mut pt = Vec::with_capacity(1 + body.len());
    pt.push(rtype);
    pt.extend_from_slice(body);
    let ct = link.tx.encrypt(&[], &pt).ok_or(MeshError::Seal)?;
    push_frame(out, &ct);
    Ok(())
}

fn open_record(link: &mut Link, frame: &[u8]) -> Result<(u8, Vec<u8>), MeshError> {
    let pt = link.rx.decrypt(&[], frame).ok_or(MeshError::Decrypt)?;
    if pt.is_empty() {
        return Err(MeshError::Frame);
    }
    Ok((pt[0], pt[1..].to_vec()))
}

/// Finish a handshake: SAS, peer static, transport states.
fn finish(hs: Handshake) -> Result<(Link, u32), MeshError> {
    if !hs.is_finished() {
        return Err(MeshError::Handshake);
    }
    let sas = mesh_crypto::sas(&hs.handshake_hash());
    let peer = hs.remote_static().ok_or(MeshError::Handshake)?;
    let (tx, rx) = hs.split();
    Ok((Link { tx, rx, peer }, sas))
}

/// Seal `text` to `peer` as a bucket-0 AFR-0009 bundle from this node.
fn seal_to(sk: &[u8; 32], pk: &[u8; 32], peer: &[u8; 32], text: &[u8]) -> Result<Vec<u8>, MeshError> {
    let mut dh = Secret(mesh_hpke::x25519_dh(sk, peer));
    let mut root = Secret(mesh_crypto::contact_root(pk, peer, &dh.0));
    crate::drbg::wipe(&mut dh.0);
    let created = now_unix();
    let tag = mesh_crypto::recipient_tag(&root.0, pk, peer, mesh_crypto::tag_period(created));
    crate::drbg::wipe(&mut root.0);
    let mut bid = [0u8; 16];
    crate::rng::fill_secret(&mut bid).map_err(|_| MeshError::NoSecretRandom)?;
    let ske = secret32()?;
    mesh_hpke::bundle_seal(BUCKET, TTL, &bid, &tag, created + 86_400, created, 1, text, &ske.0, sk, peer)
        .map_err(|_| MeshError::Seal)
}

/// Open a bundle addressed to this node from `peer`, checking the recipient tag for the bundle's own period.
fn open_from(sk: &[u8; 32], pk: &[u8; 32], peer: &[u8; 32], bundle: &[u8]) -> Result<(u8, u64, Vec<u8>), MeshError> {
    let (mtype, created, body) = mesh_hpke::bundle_open(bundle, sk, peer).ok_or(MeshError::Open)?;
    // AFR-0009 section 6 step 3 (caller's part): the tag must be the one this contact pair derives for the
    // bundle's period. The header is authenticated as AAD, so a tag that matches here was sealed by `peer`.
    let dh = Secret(mesh_hpke::x25519_dh(sk, peer));
    let root = Secret(mesh_crypto::contact_root(pk, peer, &dh.0));
    let want = mesh_crypto::recipient_tag(&root.0, peer, pk, mesh_crypto::tag_period(created));
    drop((dh, root));
    if bundle.len() < 44 || bundle[20..36] != want {
        return Err(MeshError::Tag);
    }
    let expires = u64::from_le_bytes(bundle[36..44].try_into().map_err(|_| MeshError::Frame)?);
    if expires <= created {
        return Err(MeshError::Tag);
    }
    Ok((mtype, created, body))
}

// ---------------------------------------------------------------------------------------------------------------
// The responder (`mesh listen`): XX responder, then waits for one BUNDLE, opens it, answers CLOSE.

enum RState {
    WaitM1,
    WaitM3(Handshake),
    Up(Link),
    Done,
}

/// `mesh listen [port] [secs]`: accept one link on `port` (default 4401) and open the bundle it carries.
pub fn listen(port: u16, wait_ms: u64) -> bool {
    let (sk, pk) = match identity() {
        Ok(k) => k,
        Err(e) => {
            crate::kprintln!("[mesh] listen REFUSED: {:?} (secrets need a healthy hardware entropy source)", e);
            return false;
        }
    };
    crate::kprintln!("[mesh] listening on TCP {} as {}", port, s(&hex8(&pk)));
    let mut framer = Framer::new();
    let mut st = RState::WaitM1;
    let mut opened = false;
    let mut handler = |input: &[u8], out: &mut Vec<u8>| -> bool {
        framer.push(input);
        loop {
            let f = match framer.next() {
                Ok(Some(f)) => f,
                Ok(None) => return !matches!(st, RState::Done),
                Err(e) => {
                    crate::kprintln!("[mesh] link FAILED: {:?}", e);
                    st = RState::Done;
                    return false;
                }
            };
            let step: Result<bool, MeshError> = (|| -> Result<bool, MeshError> {
                match core::mem::replace(&mut st, RState::Done) {
                    RState::WaitM1 => {
                        let e = secret32()?;
                        let mut hs = Handshake::xx_responder(sk.0, e.0, &prologue());
                        let mut pl = Vec::new();
                        hs.read_message(&f, &mut pl).map_err(|_| MeshError::Handshake)?;
                        let mut m2 = Vec::new();
                        hs.write_message(&[], &mut m2).map_err(|_| MeshError::Handshake)?;
                        push_frame(out, &m2);
                        st = RState::WaitM3(hs);
                        Ok(true)
                    }
                    RState::WaitM3(mut hs) => {
                        let mut pl = Vec::new();
                        hs.read_message(&f, &mut pl).map_err(|_| MeshError::Handshake)?;
                        let (link, sas) = finish(hs)?;
                        crate::kprintln!(
                            "[mesh] link UP (Noise XX responder) peer {} SAS {:06}",
                            s(&hex8(&link.peer)), sas
                        );
                        st = RState::Up(link);
                        Ok(true)
                    }
                    RState::Up(mut link) => {
                        let (rtype, body) = open_record(&mut link, &f)?;
                        match rtype {
                            REC_BUNDLE => {
                                let (mtype, created, mut text) = open_from(&sk.0, &pk, &link.peer, &body)?;
                                crate::kprintln!(
                                    "[mesh] bundle OPENED from {} ({} bytes sealed, type {}, created {}): \"{}\"",
                                    s(&hex8(&link.peer)), body.len(), mtype, created, shown(&text)
                                );
                                crate::drbg::wipe(&mut text);
                                opened = true;
                                push_record(out, &mut link, REC_CLOSE, &[0x00])?;
                                Ok(false)
                            }
                            REC_PING => {
                                push_record(out, &mut link, REC_PING, &[])?;
                                st = RState::Up(link);
                                Ok(true)
                            }
                            REC_CLOSE => Ok(false),
                            _ => Err(MeshError::Unexpected),
                        }
                    }
                    RState::Done => Err(MeshError::Unexpected),
                }
            })();
            match step {
                Ok(true) => continue,
                Ok(false) => return false,
                Err(e) => {
                    crate::kprintln!("[mesh] link FAILED: {:?}", e);
                    st = RState::Done;
                    return false;
                }
            }
        }
    };
    crate::virtio::net_tcp_serve_with(port, wait_ms, 1, Some(&mut handler));
    crate::kprintln!("[mesh] listen done: {}", if opened { "bundle opened" } else { "NO bundle opened" });
    opened
}

// ---------------------------------------------------------------------------------------------------------------
// The initiator (`mesh send`): XX initiator, then sends one sealed BUNDLE and waits for CLOSE.

enum IState {
    WaitM2(Handshake),
    WaitClose(Link),
    Done,
}

/// `mesh send <ip>:<port> <text>`: pair with whoever listens there (XX) and send them `text`, sealed.
pub fn send(ip: [u8; 4], port: u16, text: &[u8], wait_ms: u64) -> bool {
    let (sk, pk) = match identity() {
        Ok(k) => k,
        Err(e) => {
            crate::kprintln!("[mesh] send REFUSED: {:?} (secrets need a healthy hardware entropy source)", e);
            return false;
        }
    };
    let max_body = mesh_crypto::BUCKETS[BUCKET] - 104;
    if text.is_empty() || text.len() > max_body {
        crate::kprintln!("[mesh] send: the message must be 1..={} bytes in Phase 0 (bucket 0)", max_body);
        return false;
    }
    let e = match secret32() {
        Ok(e) => e,
        Err(err) => {
            crate::kprintln!("[mesh] send REFUSED: {:?}", err);
            return false;
        }
    };
    let mut hs = Handshake::xx_initiator(sk.0, e.0, &prologue());
    let mut m1 = Vec::new();
    if hs.write_message(&[], &mut m1).is_err() {
        crate::kprintln!("[mesh] send: handshake message 1 failed");
        return false;
    }
    let mut first = Vec::new();
    push_frame(&mut first, &m1);
    crate::kprintln!(
        "[mesh] dialing {}.{}.{}.{}:{} as {}",
        ip[0], ip[1], ip[2], ip[3], port, s(&hex8(&pk))
    );
    let mut framer = Framer::new();
    let mut st = IState::WaitM2(hs);
    let mut confirmed = false;
    let mut handler = |input: &[u8], out: &mut Vec<u8>| -> bool {
        framer.push(input);
        loop {
            let f = match framer.next() {
                Ok(Some(f)) => f,
                Ok(None) => return !matches!(st, IState::Done),
                Err(e) => {
                    crate::kprintln!("[mesh] link FAILED: {:?}", e);
                    st = IState::Done;
                    return false;
                }
            };
            let step: Result<bool, MeshError> = (|| -> Result<bool, MeshError> { match core::mem::replace(&mut st, IState::Done) {
                IState::WaitM2(mut hs) => {
                    let mut pl = Vec::new();
                    hs.read_message(&f, &mut pl).map_err(|_| MeshError::Handshake)?;
                    let mut m3 = Vec::new();
                    hs.write_message(&[], &mut m3).map_err(|_| MeshError::Handshake)?;
                    let (mut link, sas) = finish(hs)?;
                    crate::kprintln!(
                        "[mesh] link UP (Noise XX initiator) peer {} SAS {:06}",
                        s(&hex8(&link.peer)), sas
                    );
                    let bundle = seal_to(&sk.0, &pk, &link.peer, text)?;
                    push_frame(out, &m3);
                    push_record(out, &mut link, REC_BUNDLE, &bundle)?;
                    crate::kprintln!(
                        "[mesh] bundle SEALED to {} ({} bytes, bucket {}), sent as a BUNDLE record",
                        s(&hex8(&link.peer)), bundle.len(), BUCKET
                    );
                    st = IState::WaitClose(link);
                    Ok(true)
                }
                IState::WaitClose(mut link) => {
                    let (rtype, _) = open_record(&mut link, &f)?;
                    if rtype == REC_CLOSE {
                        crate::kprintln!("[mesh] peer CLOSED the link after the bundle");
                        confirmed = true;
                        Ok(false)
                    } else {
                        Err(MeshError::Unexpected)
                    }
                }
                IState::Done => Err(MeshError::Unexpected),
            } })();
            match step {
                Ok(true) => continue,
                Ok(false) => return false,
                Err(e) => {
                    crate::kprintln!("[mesh] link FAILED: {:?}", e);
                    st = IState::Done;
                    return false;
                }
            }
        }
    };
    crate::virtio::net_tcp_dial_with(ip, port, &first, wait_ms, &mut handler);
    crate::kprintln!("[mesh] send done: {}", if confirmed { "delivered" } else { "NOT confirmed" });
    confirmed
}

// ---------------------------------------------------------------------------------------------------------------
// `mesh selftest`: the in-kernel checks the review asked for, run on the real kernel build.

fn check(name: &str, ok: bool, pass: &mut u32, total: &mut u32) {
    *total += 1;
    if ok {
        *pass += 1;
    }
    crate::kprintln!("[mesh] selftest {}: {}", name, if ok { "PASS" } else { "FAIL" });
}

/// Run the XX handshake between two in-memory nodes; returns (link_i, link_r, sas_i, sas_r) or None.
fn xx_pair(a: [u8; 32], ea: [u8; 32], b: [u8; 32], eb: [u8; 32], corrupt_m2: bool) -> Option<(Link, Link, u32, u32)> {
    let p = prologue();
    let mut i = Handshake::xx_initiator(a, ea, &p);
    let mut r = Handshake::xx_responder(b, eb, &p);
    let mut m = Vec::new();
    let mut pl = Vec::new();
    i.write_message(&[], &mut m).ok()?;
    r.read_message(&m, &mut pl).ok()?;
    m.clear();
    r.write_message(&[], &mut m).ok()?;
    if corrupt_m2 {
        let last = m.len() - 1;
        m[last] ^= 1;
    }
    i.read_message(&m, &mut pl).ok()?;
    m.clear();
    i.write_message(&[], &mut m).ok()?;
    r.read_message(&m, &mut pl).ok()?;
    let (li, si) = finish(i).ok()?;
    let (lr, sr) = finish(r).ok()?;
    Some((li, lr, si, sr))
}

pub fn selftest() -> bool {
    let (mut pass, mut total) = (0u32, 0u32);

    // Review S3: one SHA-256 in the kernel, or proof the two agree. mesh-crypto's SHA-256 vs the kernel's.
    let mut long = [0u8; 1000];
    for (i, b) in long.iter_mut().enumerate() {
        *b = (i * 31 + 7) as u8;
    }
    let vecs: [&[u8]; 5] = [b"", b"abc", b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", &long[..64], &long];
    let same = vecs.iter().all(|v| mesh_crypto::sha256::digest(v) == crate::sha256::digest(v));
    check("sha256-kernel-vs-mesh-crypto", same && crate::sha256::self_test(), &mut pass, &mut total);

    // The DRBG the keys come from.
    let st = crate::rng::status();
    check("drbg-kat", st.kat_ok, &mut pass, &mut total);

    // Keys and ephemerals from fill_secret (review S4: fresh, never zero, two handshakes' ephemerals differ).
    let keys: Result<[Secret; 6], MeshError> = (|| -> Result<[Secret; 6], MeshError> {
        Ok([secret32()?, secret32()?, secret32()?, secret32()?, secret32()?, secret32()?])
    })();
    let ks = match keys {
        Ok(k) => k,
        Err(e) => {
            crate::kprintln!("[mesh] selftest: no secret randomness ({:?}); boot with a virtio-rng device", e);
            check("fill-secret", false, &mut pass, &mut total);
            crate::kprintln!("[mesh] selftest {}/{} PASS", pass, total);
            return false;
        }
    };
    let k: [[u8; 32]; 6] = [ks[0].0, ks[1].0, ks[2].0, ks[3].0, ks[4].0, ks[5].0];
    let distinct = (0..6).all(|i| (0..6).all(|j| i == j || k[i] != k[j]));
    check("fresh-ephemerals-distinct", distinct, &mut pass, &mut total);

    // XX in memory: SAS agree, static keys learned, transport round trip both ways.
    let (a, b) = (k[0], k[1]);
    let (pa, pb) = (mesh_noise::public_key(&a), mesh_noise::public_key(&b));
    let xx = xx_pair(a, k[2], b, k[3], false);
    let ok = match xx {
        Some((mut li, mut lr, si, sr)) => {
            let mut fr = Vec::new();
            let r1 = push_record(&mut fr, &mut li, REC_PING, b"habari").is_ok();
            let got = open_record(&mut lr, &fr[2..]).ok();
            let mut fr2 = Vec::new();
            let r2 = push_record(&mut fr2, &mut lr, REC_PING, b"salama").is_ok();
            let got2 = open_record(&mut li, &fr2[2..]).ok();
            si == sr && li.peer == pb && lr.peer == pa && r1 && r2
                && got == Some((REC_PING, b"habari".to_vec()))
                && got2 == Some((REC_PING, b"salama".to_vec()))
        }
        None => false,
    };
    check("noise-xx-handshake-and-transport", ok, &mut pass, &mut total);

    // Second XX with different ephemerals: a different SAS (the handshake hash depends on the ephemerals).
    let sas_differs = match (xx_pair(a, k[2], b, k[3], false), xx_pair(a, k[4], b, k[5], false)) {
        (Some((_, _, s1, _)), Some((_, _, s2, _))) => s1 != s2,
        _ => false,
    };
    check("second-handshake-new-sas", sas_differs, &mut pass, &mut total);

    // Negative: one flipped byte in message 2 must fail the handshake.
    check("noise-xx-flipped-byte-rejected", xx_pair(a, k[2], b, k[3], true).is_none(), &mut pass, &mut total);

    // KK (contacts): both statics known in advance.
    let kk_ok = (|| -> Option<bool> {
        let p = prologue();
        let mut i = Handshake::kk_initiator(a, k[4], pb, &p);
        let mut r = Handshake::kk_responder(b, k[5], pa, &p);
        let mut m = Vec::new();
        let mut pl = Vec::new();
        i.write_message(&[], &mut m).ok()?;
        r.read_message(&m, &mut pl).ok()?;
        m.clear();
        r.write_message(&[], &mut m).ok()?;
        i.read_message(&m, &mut pl).ok()?;
        Some(i.is_finished() && r.is_finished() && i.handshake_hash() == r.handshake_hash())
    })()
    .unwrap_or(false);
    check("noise-kk-handshake", kk_ok, &mut pass, &mut total);

    // KK with the wrong responder static fails.
    let kk_wrong = (|| -> Option<()> {
        let p = prologue();
        let mut i = Handshake::kk_initiator(a, k[4], pa, &p); // wrong: A's own key as B's
        let mut r = Handshake::kk_responder(b, k[5], pa, &p);
        let mut m = Vec::new();
        let mut pl = Vec::new();
        i.write_message(&[], &mut m).ok()?;
        r.read_message(&m, &mut pl).ok()?;
        Some(())
    })()
    .is_none();
    check("noise-kk-wrong-static-rejected", kk_wrong, &mut pass, &mut total);

    // Bundle: seal A -> B, open as B; a flipped ciphertext byte and the wrong sender both fail.
    let sealed = seal_to(&a, &pa, &pb, b"Habari kutoka AfriSovera");
    let (rt, tamper, wrong) = match &sealed {
        Ok(bun) => {
            let rt = open_from(&b, &pb, &pa, bun);
            let mut t = bun.clone();
            let last = t.len() - 1;
            t[last] ^= 1;
            let tamper = open_from(&b, &pb, &pa, &t).is_err();
            let wrong = open_from(&b, &pb, &pb, bun).is_err();
            (rt.map(|x| x.2).ok(), tamper, wrong)
        }
        Err(_) => (None, false, false),
    };
    check("bundle-seal-open", rt.as_deref() == Some(&b"Habari kutoka AfriSovera"[..]), &mut pass, &mut total);
    check("bundle-tamper-rejected", tamper, &mut pass, &mut total);
    check("bundle-wrong-sender-rejected", wrong, &mut pass, &mut total);
    check(
        "bundle-is-bucket-0",
        sealed.as_ref().map(|v| v.len() == mesh_crypto::BUCKETS[0]).unwrap_or(false),
        &mut pass,
        &mut total,
    );

    // Review M1: `forget` must wipe the secret bytes INSIDE the static, not a moved copy. Read the static's
    // own bytes through a raw pointer before and after; a "new identity differs" check cannot see this.
    let forget_wipes = identity().is_ok()
        && match identity_secret_ptr() {
            Some(ptr) => {
                // SAFETY: `ptr` points at 32 bytes inside the IDENTITY static, which lives for the whole boot;
                // volatile reads, so the compiler cannot reuse values read before `forget`.
                let read = || -> [u8; 32] {
                    let mut b = [0u8; 32];
                    for (i, x) in b.iter_mut().enumerate() {
                        *x = unsafe { core::ptr::read_volatile(ptr.add(i)) };
                    }
                    b
                };
                let before = read();
                let forgot = forget();
                let after = read();
                before != [0u8; 32] && forgot && after == [0u8; 32]
            }
            None => false,
        };
    check("forget-wipes-identity-in-static", forget_wipes, &mut pass, &mut total);

    // F1: the wipes run on the real kernel build. Each value is dropped in place, then its bytes are read.
    let mut sec = core::mem::ManuallyDrop::new(Secret([0xa5; 32]));
    // SAFETY: dropped exactly once; afterwards only a plain byte array is read and ManuallyDrop never re-drops.
    unsafe { core::ptr::drop_in_place(&mut *sec) };
    check("secret-drop-wipes", sec.0 == [0u8; 32], &mut pass, &mut total);
    let mut d = core::mem::ManuallyDrop::new(crate::drbg::HmacDrbg::instantiate(&k[0], &k[1], b"selftest"));
    let mut probe = [0u8; 32];
    let generated = d.generate(&mut probe, b"").is_ok() && probe != [0u8; 32];
    // SAFETY: as above.
    unsafe { core::ptr::drop_in_place(&mut *d) };
    check("drbg-drop-wipes", generated && d.is_wiped(), &mut pass, &mut total);

    // F4: peer text with a screen-clearing escape, a fake new line, a bidi override and a closing quote.
    let nasty = "hi\x1b[2J\r\n[mesh] send done: delivered \u{202e}x\"".as_bytes();
    check(
        "peer-text-console-safe",
        shown(nasty) == "hi\\x1b[2J\\x0d\\x0a[mesh] send done: delivered \\u{202e}x\\\""
            && shown(b"Habari-from-A") == "Habari-from-A",
        &mut pass,
        &mut total,
    );

    // The plain copies of the six test keys (the `Secret`s in `ks` wipe themselves when dropped).
    let mut k = k;
    for x in k.iter_mut() {
        crate::drbg::wipe(x);
    }
    crate::kprintln!("[mesh] selftest {}/{} PASS", pass, total);
    pass == total
}

/// `mesh id`: this node's identity public key.
pub fn show_id() {
    match identity() {
        Ok((_sk, pk)) => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut h = [0u8; 64];
            for (i, &b) in pk.iter().enumerate() {
                h[i * 2] = HEX[(b >> 4) as usize];
                h[i * 2 + 1] = HEX[(b & 15) as usize];
            }
            crate::kprintln!("[mesh] identity x25519 {}", s(&h));
        }
        Err(e) => crate::kprintln!("[mesh] identity REFUSED: {:?}", e),
    }
}
