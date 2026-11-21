//! AfriSovera's own Wi-Fi stack (WIFI_PLAN.md in the Wi-Fi engineer's notes; architect's verdict 14 Nov 2026).
//!
//! PC kernel only for now: reached from pc/main.rs alone, so the ARM image does not change. Nothing here touches
//! hardware or prints, so the host tests (wlan-crypto/tests/stack.rs) and the fuzz-host harness compile these
//! files by `#[path]`, unchanged.
//!
//! W1: the cryptographic primitives (crypto.rs) and their known-answer tests (crypto_kat.rs, generated from the
//! published files by scripts/wifi-kat-fetch.sh).
//! W2: frames (frame.rs: header, management bodies, elements, RSN), the scan table (bss.rs) and the conversion
//! between 802.11 data frames and Ethernet frames (llc.rs).
//! W3: CCMP (ccmp.rs), EAPOL-Key frames (eapol.rs) and the WPA2-PSK supplicant (supp.rs).
//! W4: the station (mlme.rs): choosing a network, joining it, data both ways, over a `Radio` the driver implements.
//! The Intel driver comes in W5 to W7.

pub mod bss;
pub mod ccmp;
pub mod crypto;
pub mod crypto_kat;
pub mod eapol;
pub mod frame;
pub mod llc;
pub mod mlme;
pub mod supp;
