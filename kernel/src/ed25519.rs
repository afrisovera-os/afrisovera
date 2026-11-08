//! Ed25519 **signature verification** (no signing) for AfriStore code-signing —
//! proving *who* published an app, the authorship layer above SHA-256 integrity.
//!
//! This is a faithful `no_std` port of the compact, well-audited **TweetNaCl**
//! Ed25519 (D. J. Bernstein et al.): curve25519 field arithmetic over
//! `p = 2^255 - 19` (16 × 16-bit limbs in `i64`), twisted-Edwards points in
//! extended coordinates, and the standard verify equation
//! `[S]B == R + [k]A`, `k = SHA-512(R‖A‖M) mod L`. Publishers sign off-device with
//! their private key; the device only ever verifies.
//!
//! Validated against RFC 8032 Ed25519 Test 1 (see `self_test`).

use crate::sha512;
use alloc::vec::Vec;

type Gf = [i64; 16];

const GF0: Gf = [0; 16];
const GF1: Gf = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
#[rustfmt::skip]
const D: Gf = [0x78a3, 0x1359, 0x4dca, 0x75eb, 0xd8ab, 0x4141, 0x0a4d, 0x0070, 0xe898, 0x7779, 0x4079, 0x8cc7, 0xfe73, 0x2b6f, 0x6cee, 0x5203];
#[rustfmt::skip]
const D2: Gf = [0xf159, 0x26b2, 0x9b94, 0xebd6, 0xb156, 0x8283, 0x149a, 0x00e0, 0xd130, 0xeef3, 0x80f2, 0x198e, 0xfce7, 0x56df, 0xd9dc, 0x2406];
#[rustfmt::skip]
const X: Gf = [0xd51a, 0x8f25, 0x2d60, 0xc956, 0xa7b2, 0x9525, 0xc760, 0x692c, 0xdc5c, 0xfdd6, 0xe231, 0xc0a4, 0x53fe, 0xcd6e, 0x36d3, 0x2169];
#[rustfmt::skip]
const Y: Gf = [0x6658, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666];
#[rustfmt::skip]
const SQRTM1: Gf = [0xa0b0, 0x4a0e, 0x1b27, 0xc4ee, 0xe478, 0xad2f, 0x1806, 0x2f43, 0xd7a7, 0x3dfb, 0x0099, 0x2b4d, 0xdf0b, 0x4fc1, 0x2480, 0x2b83];
#[rustfmt::skip]
const L: [i64; 32] = [0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10];

fn fe_car(o: &mut Gf) {
    for i in 0..16 {
        o[i] += 1i64 << 16;
        let c = o[i] >> 16;
        if i < 15 {
            o[i + 1] += c - 1;
        } else {
            o[0] += 38 * (c - 1);
        }
        o[i] -= c << 16;
    }
}

fn fe_a(x: &Gf, y: &Gf) -> Gf {
    let mut o = [0i64; 16];
    for i in 0..16 {
        o[i] = x[i] + y[i];
    }
    o
}

fn fe_z(x: &Gf, y: &Gf) -> Gf {
    let mut o = [0i64; 16];
    for i in 0..16 {
        o[i] = x[i] - y[i];
    }
    o
}

fn fe_m(x: &Gf, y: &Gf) -> Gf {
    let mut t = [0i64; 31];
    for i in 0..16 {
        for j in 0..16 {
            t[i + j] += x[i] * y[j];
        }
    }
    for i in 0..15 {
        t[i] += 38 * t[i + 16];
    }
    let mut o = [0i64; 16];
    o[..16].copy_from_slice(&t[..16]);
    fe_car(&mut o);
    fe_car(&mut o);
    o
}

fn fe_s(x: &Gf) -> Gf {
    fe_m(x, x)
}

/// Multiplicative inverse via x^(p-2).
fn fe_inv(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=253).rev() {
        c = fe_s(&c);
        if a != 2 && a != 4 {
            c = fe_m(&c, i);
        }
    }
    c
}

/// x^((p-5)/8), used in the square-root for point decompression.
fn fe_pow2523(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=250).rev() {
        c = fe_s(&c);
        if a != 1 {
            c = fe_m(&c, i);
        }
    }
    c
}

/// Constant-time conditional swap of two field elements when b == 1.
fn fe_sel(p: &mut Gf, q: &mut Gf, b: i64) {
    let mask = !(b - 1);
    for i in 0..16 {
        let t = mask & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}

fn pack25519(o: &mut [u8; 32], n: &Gf) {
    let mut t = *n;
    fe_car(&mut t);
    fe_car(&mut t);
    fe_car(&mut t);
    for _ in 0..2 {
        let mut m = [0i64; 16];
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        fe_sel(&mut t, &mut m, 1 - b);
    }
    for i in 0..16 {
        o[2 * i] = (t[i] & 0xff) as u8;
        o[2 * i + 1] = (t[i] >> 8) as u8;
    }
}

fn unpack25519(n: &[u8; 32]) -> Gf {
    let mut o = [0i64; 16];
    for i in 0..16 {
        o[i] = (n[2 * i] as i64) + ((n[2 * i + 1] as i64) << 8);
    }
    o[15] &= 0x7fff;
    o
}

fn par25519(a: &Gf) -> u8 {
    let mut d = [0u8; 32];
    pack25519(&mut d, a);
    d[0] & 1
}

/// True if the two field elements differ (canonical-packed comparison).
fn neq25519(a: &Gf, b: &Gf) -> bool {
    let mut c = [0u8; 32];
    let mut d = [0u8; 32];
    pack25519(&mut c, a);
    pack25519(&mut d, b);
    c != d
}

// Twisted-Edwards point in extended coordinates: [X, Y, Z, T].
type Point = [Gf; 4];

fn point_add(p: &mut Point, q: &Point) {
    let a = fe_z(&p[1], &p[0]);
    let t = fe_z(&q[1], &q[0]);
    let a = fe_m(&a, &t);
    let b = fe_a(&p[0], &p[1]);
    let t = fe_a(&q[0], &q[1]);
    let b = fe_m(&b, &t);
    let c = fe_m(&p[3], &q[3]);
    let c = fe_m(&c, &D2);
    let d = fe_m(&p[2], &q[2]);
    let d = fe_a(&d, &d);
    let e = fe_z(&b, &a);
    let f = fe_z(&d, &c);
    let g = fe_a(&d, &c);
    let h = fe_a(&b, &a);
    p[0] = fe_m(&e, &f);
    p[1] = fe_m(&h, &g);
    p[2] = fe_m(&g, &f);
    p[3] = fe_m(&e, &h);
}

fn point_cswap(p: &mut Point, q: &mut Point, b: u8) {
    for i in 0..4 {
        fe_sel(&mut p[i], &mut q[i], b as i64);
    }
}

fn scalarmult(base: &Point, s: &[u8; 32]) -> Point {
    let mut p: Point = [GF0, GF1, GF1, GF0];
    let mut q = *base;
    for i in (0..=255).rev() {
        let b = (s[i >> 3] >> (i & 7)) & 1;
        point_cswap(&mut p, &mut q, b);
        point_add(&mut q, &p);
        let pc = p;
        point_add(&mut p, &pc); // double p
        point_cswap(&mut p, &mut q, b);
    }
    p
}

fn scalarbase(s: &[u8; 32]) -> Point {
    let q: Point = [X, Y, GF1, fe_m(&X, &Y)];
    scalarmult(&q, s)
}

fn pack_point(p: &Point) -> [u8; 32] {
    let zi = fe_inv(&p[2]);
    let tx = fe_m(&p[0], &zi);
    let ty = fe_m(&p[1], &zi);
    let mut r = [0u8; 32];
    pack25519(&mut r, &ty);
    r[31] ^= par25519(&tx) << 7;
    r
}

/// Decompress a public key to the *negated* point -A (as TweetNaCl does), so the
/// verify step can compute [S]B + [k](-A). Returns None if the key is not on-curve.
fn unpackneg(pk: &[u8; 32]) -> Option<Point> {
    let mut r: Point = [GF0, unpack25519(pk), GF1, GF0];
    let num = fe_s(&r[1]); // y^2
    let den = fe_m(&num, &D); // d*y^2
    let num = fe_z(&num, &r[2]); // y^2 - 1
    let den = fe_a(&r[2], &den); // d*y^2 + 1

    let den2 = fe_s(&den);
    let den4 = fe_s(&den2);
    let den6 = fe_m(&den4, &den2);
    let t = fe_m(&den6, &num);
    let t = fe_m(&t, &den);

    let t = fe_pow2523(&t);
    let t = fe_m(&t, &num);
    let t = fe_m(&t, &den);
    let t = fe_m(&t, &den);
    r[0] = fe_m(&t, &den);

    let chk = fe_s(&r[0]);
    let chk = fe_m(&chk, &den);
    if neq25519(&chk, &num) {
        r[0] = fe_m(&r[0], &SQRTM1);
    }

    let chk = fe_s(&r[0]);
    let chk = fe_m(&chk, &den);
    if neq25519(&chk, &num) {
        return None;
    }

    if par25519(&r[0]) == (pk[31] >> 7) {
        r[0] = fe_z(&GF0, &r[0]);
    }
    r[3] = fe_m(&r[0], &r[1]);
    Some(r)
}

/// Reduce a 64-byte little-endian scalar mod L, returning the 32-byte result.
fn reduce(h: &[u8; 64]) -> [u8; 32] {
    let mut x = [0i64; 64];
    for i in 0..64 {
        x[i] = h[i] as i64;
    }
    let mut r = [0u8; 32];
    for i in (32..=63).rev() {
        let mut carry = 0i64;
        let mut j = i - 32;
        while j < i - 12 {
            x[j] += carry - 16 * x[i] * L[j - (i - 32)];
            carry = (x[j] + 128) >> 8;
            x[j] -= carry << 8;
            j += 1;
        }
        x[j] += carry;
        x[i] = 0;
    }
    let mut carry = 0i64;
    for j in 0..32 {
        x[j] += carry - (x[31] >> 4) * L[j];
        carry = x[j] >> 8;
        x[j] &= 255;
    }
    for j in 0..32 {
        x[j] -= carry * L[j];
    }
    for i in 0..32 {
        x[i + 1] += x[i] >> 8;
        r[i] = (x[i] & 255) as u8;
    }
    r
}

/// Verify an Ed25519 signature `sig` (R‖S) over `msg` under `pubkey`.
pub fn verify(pubkey: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let neg_a = match unpackneg(pubkey) {
        Some(p) => p,
        None => return false,
    };

    // k = SHA-512(R || A || M) mod L
    let mut buf = Vec::with_capacity(64 + msg.len());
    buf.extend_from_slice(&sig[0..32]); // R
    buf.extend_from_slice(pubkey); // A
    buf.extend_from_slice(msg); // M
    let k = reduce(&sha512::digest(&buf));

    let p = scalarmult(&neg_a, &k); // [k](-A)
    let mut s = [0u8; 32];
    s.copy_from_slice(&sig[32..64]);
    let mut r = scalarbase(&s); // [S]B
    point_add(&mut r, &p); // [S]B + [k](-A) = [S]B - [k]A
    let check = pack_point(&r);

    // Valid iff the recomputed point equals R.
    check[..] == sig[0..32]
}

/// RFC 8032 Ed25519 Test 1: a known-good (pubkey, empty message, signature) must
/// verify, and any bit flip must fail. Returns true iff both hold.
pub fn self_test() -> bool {
    let pk: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
        0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
    ];
    let sig: [u8; 64] = [
        0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e, 0x82, 0x8a,
        0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65, 0x22, 0x49, 0x01, 0x55,
        0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e, 0x39, 0x70, 0x1c, 0xf9, 0xb4, 0x6b,
        0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24, 0x65, 0x51, 0x41, 0x43, 0x8e, 0x7a, 0x10, 0x0b,
    ];
    let good = verify(&pk, b"", &sig);
    let mut bad_sig = sig;
    bad_sig[0] ^= 0x01;
    let bad = verify(&pk, b"", &bad_sig);
    good && !bad
}
