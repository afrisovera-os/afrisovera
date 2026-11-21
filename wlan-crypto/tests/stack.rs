//! The kernel's Wi-Fi stack (kernel/src/wlan/), compiled here unchanged by `#[path]`: every `#[cfg(test)]` unit
//! test inside those files runs on the host (frames, the scan table, data frames to Ethernet and back), and the
//! kernel's own known-answer runner (crypto.rs `run_kind`) is run over the generated constants.
#[path = "../../kernel/src/wlan/mod.rs"]
#[allow(dead_code, unused_imports)]
mod wlan;

#[test]
fn kernel_kat_runner_passes_every_family() {
    let mut total = 0;
    for k in wlan::crypto::KINDS {
        match wlan::crypto::run_kind(k) {
            Ok(n) => total += n,
            Err(i) => panic!("{} vector {} failed", k.name(), i + 1),
        }
    }
    assert_eq!(total, 39);
}

#[test]
fn secret_wipes_on_drop() {
    // the wipe itself: a buffer handed to wipe() is all zero after
    let mut b = [0xA5u8; 64];
    wlan::crypto::wipe(&mut b);
    assert!(b.iter().all(|&x| x == 0));
    let mut s = wlan::crypto::Secret([7u8; 32]);
    s.wipe();
    assert_eq!(s.0, [0u8; 32]);
}

#[test]
fn typed_key_buffers_are_wiped_kept_or_refused() {
    // the architect's rule (14 Nov 2026): 8 to 63 printable ASCII is a passphrase, 64 hex digits the PSK itself; the
    // typed bytes are zero afterwards in every case
    let ssid = b"IEEE";
    let mut typed = *b"password";
    let k = wlan::crypto::pmk_from_typed(&mut typed, ssid).expect("a good passphrase");
    assert_eq!(k.0, wlan_crypto::psk_from_passphrase(b"password", ssid).unwrap(), "the passphrase's PMK");
    assert_eq!(typed, [0u8; 8], "passphrase kept: buffer not wiped");
    let mut short = *b"seven77";
    assert!(wlan::crypto::pmk_from_typed(&mut short, ssid).is_err());
    assert_eq!(short, [0u8; 7], "refused passphrase: buffer not wiped");
    let mut ctrl = *b"tab\there!";
    assert!(wlan::crypto::pmk_from_typed(&mut ctrl, ssid).is_err(), "a control character is not printable ASCII");
    assert_eq!(ctrl, [0u8; 9]);
    let hex = b"f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12E";
    let mut h = *hex;
    let k = wlan::crypto::pmk_from_typed(&mut h, ssid).expect("64 hex digits");
    assert_eq!(&k.0[..4], &[0xf4, 0x2c, 0x6f, 0xc5]);
    assert_eq!(k.0[31], 0x2E, "upper-case hex is read too");
    assert_eq!(h, [0u8; 64], "hex PSK kept: buffer not wiped");
    let mut bad = *hex;
    bad[10] = b'g';
    assert!(wlan::crypto::pmk_from_typed(&mut bad, ssid).is_err());
    assert_eq!(bad, [0u8; 64], "refused hex PSK: buffer not wiped");
    let mut long = [b'a'; 65];
    assert!(wlan::crypto::pmk_from_typed(&mut long, ssid).is_err(), "65 characters is neither");
    assert_eq!(long, [0u8; 65]);
}
