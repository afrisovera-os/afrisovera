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
//! The supplicant, the station state machine and the Intel driver come in W3 to W7.

pub mod bss;
pub mod crypto;
pub mod crypto_kat;
pub mod frame;
pub mod llc;
