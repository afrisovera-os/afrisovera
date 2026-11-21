//! The Wi-Fi radio under the station on a real PC (W4b; pc/main.rs builds this file as `air`). The test image builds
//! pc/wifisim.rs in its place, with the same interface.
//!
//! No radio yet: the Intel 7265 driver (W5) goes here, and `probe` finds it. Until then the PC has no Wi-Fi card and
//! the `wifi scan|connect|status|disconnect` commands say so.

use crate::wlan::frame::{BssInfo, Mac};
use crate::wlan::mlme::Radio;

/// A cable wins over Wi-Fi on a real PC: Wi-Fi is the card only when there is no wired one.
pub const PREFERRED: bool = false;
pub const DESCRIBE: &str = "Wi-Fi";
pub const SHORT: &str = "wifi";

#[allow(dead_code)] // made by `probe` once the 7265 driver is in (W5)
pub struct Air {
    own: Mac,
}

impl Air {
    /// The radio, if this PC has one AfriSovera drives.
    pub fn probe() -> Option<Air> {
        None
    }

    /// Time moves (ms since power-on).
    pub fn tick(&mut self, _now: u64) {}

    /// The next received frame, into `dst`.
    pub fn pop(&mut self, _dst: &mut [u8]) -> Option<usize> {
        None
    }

    /// More lines for `wifi status`.
    pub fn status_line(&self) {}
}

impl Radio for Air {
    fn mac(&self) -> Mac {
        self.own
    }
    fn tx(&mut self, _f: &[u8]) -> bool {
        false
    }
    fn start_scan(&mut self) -> bool {
        false
    }
    fn join(&mut self, _b: &BssInfo) -> bool {
        false
    }
    fn associated(&mut self, _b: &BssInfo, _aid: u16) -> bool {
        false
    }
    fn leave(&mut self) {}
}
