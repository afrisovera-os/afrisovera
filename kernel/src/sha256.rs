//! SHA-256 (FIPS 180-4) — a small, dependency-free implementation for **app
//! integrity**. AfriStore is a no-gatekeeper store, but no-gatekeeper must not mean
//! no-trust: a content hash lets anyone verify an app's bytes are exactly what the
//! publisher shipped (tamper/corruption detection). This is the first layer;
//! asymmetric code-signing (Ed25519, proving *who* published) is future work.
//!
//! Validated against the FIPS test vector: SHA-256("abc") =
//! ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61 f20015ad.

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[rustfmt::skip]
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for t in 0..16 {
        w[t] = u32::from_be_bytes([block[t * 4], block[t * 4 + 1], block[t * 4 + 2], block[t * 4 + 3]]);
    }
    for t in 16..64 {
        let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
        let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16].wrapping_add(s0).wrapping_add(w[t - 7]).wrapping_add(s1);
    }
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
        (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
    for t in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[t]).wrapping_add(w[t]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

/// SHA-256 digest of `data`.
pub fn digest(data: &[u8]) -> [u8; 32] {
    let mut h = H0;
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut block = [0u8; 64];

    let mut i = 0;
    while i + 64 <= data.len() {
        block.copy_from_slice(&data[i..i + 64]);
        compress(&mut h, &block);
        i += 64;
    }

    // Final block(s): remaining bytes, 0x80, zero pad, then 64-bit big-endian length.
    let rem = data.len() - i;
    block = [0u8; 64];
    block[..rem].copy_from_slice(&data[i..]);
    block[rem] = 0x80;
    if rem >= 56 {
        compress(&mut h, &block);
        block = [0u8; 64];
    }
    block[56..64].copy_from_slice(&bitlen.to_be_bytes());
    compress(&mut h, &block);

    let mut out = [0u8; 32];
    for (n, &word) in h.iter().enumerate() {
        out[n * 4..n * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Streaming SHA-256 (same compression function as `digest`), for HMAC in the
/// DRBG (`drbg.rs`) and the mesh, so the kernel carries ONE SHA-256 (review S3).
/// `digest(x) == { let mut s = Sha256::new(); s.update(x); s.finish() }`.
#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    fill: usize,
    total: u64,
}

impl Sha256 {
    pub const fn new() -> Self {
        Sha256 { h: H0, buf: [0; 64], fill: 0, total: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.fill > 0 {
            let take = (64 - self.fill).min(data.len());
            self.buf[self.fill..self.fill + take].copy_from_slice(&data[..take]);
            self.fill += take;
            data = &data[take..];
            if self.fill < 64 {
                return;
            }
            let b = self.buf;
            compress(&mut self.h, &b);
            self.fill = 0;
        }
        while data.len() >= 64 {
            let mut b = [0u8; 64];
            b.copy_from_slice(&data[..64]);
            compress(&mut self.h, &b);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.fill = data.len();
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bitlen = self.total.wrapping_mul(8);
        let rem = self.fill;
        let mut block = [0u8; 64];
        block[..rem].copy_from_slice(&self.buf[..rem]);
        block[rem] = 0x80;
        if rem >= 56 {
            compress(&mut self.h, &block);
            block = [0u8; 64];
        }
        block[56..64].copy_from_slice(&bitlen.to_be_bytes());
        compress(&mut self.h, &block);
        let mut out = [0u8; 32];
        for (n, &word) in self.h.iter().enumerate() {
            out[n * 4..n * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

/// F1 (review S2): an HMAC hash state carries key material (the padded key is its first block), so a
/// streaming state wipes its chaining value and buffer when dropped.
impl Drop for Sha256 {
    fn drop(&mut self) {
        for w in self.h.iter_mut() {
            // SAFETY: a valid &mut u32.
            unsafe { core::ptr::write_volatile(w, 0) };
        }
        for b in self.buf.iter_mut() {
            // SAFETY: a valid &mut u8.
            unsafe { core::ptr::write_volatile(b, 0) };
        }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}

/// Write `digest` as lowercase hex into a 64-byte buffer.
pub fn to_hex(d: &[u8; 32], out: &mut [u8; 64]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (i, &byte) in d.iter().enumerate() {
        out[i * 2] = HEX[(byte >> 4) as usize];
        out[i * 2 + 1] = HEX[(byte & 0xf) as usize];
    }
}

/// Self-test against the FIPS SHA-256("abc") vector. Returns true if correct.
pub fn self_test() -> bool {
    let want: [u8; 32] = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
        0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    ];
    digest(b"abc") == want
}
