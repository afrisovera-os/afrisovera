//! Every vector in fixtures/kat.txt, which scripts/wifi-kat-fetch.sh parsed from the published files (RFC 2202,
//! 6070, 3394, 4493, 3610 and hostapd's copy of IEEE 802.11 Annexes J.3 and J.4; sizes and sha256 in
//! fixtures/SOURCE.txt). Nothing in it was typed by hand. Each primitive is checked both ways where it has two.
use wlan_crypto::*;

fn hx(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    assert!(s.len() % 2 == 0, "odd hex {s}");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn a16(v: &[u8]) -> [u8; 16] {
    v.try_into().expect("16 bytes")
}

#[test]
fn every_published_vector() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/kat.txt"))
        .expect("fixtures/kat.txt (made by scripts/wifi-kat-fetch.sh)");
    let mut n = std::collections::BTreeMap::<&str, usize>::new();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f[0] {
            "hmac_sha1" => {
                let (k, d, t) = (hx(f[1]), hx(f[2]), hx(f[3]));
                assert_eq!(hmac_sha1(&k, &[&d]).to_vec(), t, "{line}");
                // split input gives the same answer, and the verifier agrees, also on a truncated tag
                let mid = d.len() / 2;
                assert_eq!(hmac_sha1(&k, &[&d[..mid], &d[mid..]]).to_vec(), t);
                assert!(hmac_sha1_verify(&k, &[&d], &t));
                assert!(hmac_sha1_verify(&k, &[&d], &t[..12]));
                let mut bad = t.clone();
                bad[19] ^= 0x80;
                assert!(!hmac_sha1_verify(&k, &[&d], &bad));
            }
            "pbkdf2_sha1" => {
                let (p, s, c, dk) = (hx(f[1]), hx(f[2]), f[3].parse::<u32>().unwrap(), hx(f[4]));
                let mut out = vec![0u8; dk.len()];
                pbkdf2_sha1(&p, &s, c, &mut out).unwrap();
                assert_eq!(out, dk, "{line}");
            }
            "psk" => {
                let (p, s, want) = (hx(f[1]), hx(f[2]), hx(f[3]));
                assert_eq!(psk_from_passphrase(&p, &s).unwrap().to_vec(), want, "{line}");
            }
            "prf_sha1" => {
                let (k, a, d, want) = (hx(f[1]), hx(f[2]), hx(f[3]), hx(f[4]));
                let mut out = vec![0u8; want.len()];
                prf_sha1(&k, &a, &d, &mut out).unwrap();
                assert_eq!(out, want, "{line}");
                // a shorter output is the prefix of the longer one (the PRF truncates, as 802.11 says)
                let mut short = vec![0u8; 16];
                prf_sha1(&k, &a, &d, &mut short).unwrap();
                assert_eq!(short, want[..16]);
            }
            "aes_kw" => {
                let (kek, plain, ct) = (a16(&hx(f[1])), hx(f[2]), hx(f[3]));
                let mut w = vec![0u8; plain.len() + 8];
                aes_wrap(&kek, &plain, &mut w).unwrap();
                assert_eq!(w, ct, "{line}");
                let mut u = vec![0u8; plain.len()];
                aes_unwrap(&kek, &ct, &mut u).unwrap();
                assert_eq!(u, plain);
            }
            "cmac" => {
                let (k, m, t) = (a16(&hx(f[1])), hx(f[2]), hx(f[3]));
                assert_eq!(cmac_aes128(&k, &[&m]).to_vec(), t, "{line}");
            }
            "ccm8" => {
                let (k, nonce, aad, plain, ct) = (a16(&hx(f[1])), hx(f[2]), hx(f[3]), hx(f[4]), hx(f[5]));
                let nonce: [u8; 13] = nonce.as_slice().try_into().unwrap();
                let mut buf = plain.clone();
                let tag = ccm8_encrypt(&k, &nonce, &aad, &mut buf).unwrap();
                assert_eq!(buf, ct[..plain.len()], "{line}");
                assert_eq!(tag.to_vec(), ct[plain.len()..]);
                ccm8_decrypt(&k, &nonce, &aad, &mut buf, &tag).unwrap();
                assert_eq!(buf, plain);
            }
            other => panic!("unknown vector kind {other}"),
        }
        *n.entry(f[0]).or_default() += 1;
    }
    let want = [("aes_kw", 1), ("ccm8", 12), ("cmac", 4), ("hmac_sha1", 7), ("pbkdf2_sha1", 5), ("prf_sha1", 3), ("psk", 3)];
    assert_eq!(n.into_iter().collect::<Vec<_>>(), want.to_vec(), "every kind, every vector");
}
