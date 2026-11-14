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
    assert_eq!(total, 35);
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
