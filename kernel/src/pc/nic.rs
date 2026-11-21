//! The PC's wired network card behind the emulated virtio-net device (virtio_pci.rs):
//! an Intel e1000/e1000e/igb on the PCI bus, a Realtek RTL8111/8168 family chip (rtl8169.rs),
//! or else a USB Ethernet adapter (CDC-NCM or RNDIS: a phone's USB tethering; CDC-ECM) on the xHCI (xhci.rs). The shared TCP/IP stack sees the same virtio-net device either way.
//! Or Wi-Fi (wifinet.rs, W4b): last of all on a real PC (a cable wins), first in the Wi-Fi test image, whose pretend
//! access point uses the e1000 as its uplink.

use core::sync::atomic::{AtomicU8, Ordering};

const NONE: u8 = 0;
const INTEL: u8 = 1;
const USB: u8 = 2;
const REALTEK: u8 = 3;
const WIFI: u8 = 4;
static WHICH: AtomicU8 = AtomicU8::new(NONE);

/// Pick the card: the Intel one if there is one, else a Realtek one, else a USB adapter
/// already enumerated.
pub fn init() -> bool {
    let w = if crate::wifinet::PREFERRED && crate::wifinet::init() {
        WIFI
    } else if crate::e1000::init() {
        INTEL
    } else if crate::rtl8169::init() {
        REALTEK
    } else if crate::xhci::net_present() {
        USB
    } else if !crate::wifinet::PREFERRED && crate::wifinet::init() {
        WIFI
    } else {
        NONE
    };
    WHICH.store(w, Ordering::Relaxed);
    w != NONE
}

/// How the virtio boot line names it.
pub fn describe() -> &'static str {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => "the Intel NIC (e1000)",
        REALTEK => "the Realtek NIC (rtl)",
        USB if crate::xhci::net_ncm() => "the USB Ethernet adapter (CDC-NCM)",
        USB if crate::xhci::net_rndis() => "the USB Ethernet adapter (RNDIS)",
        USB => "the USB Ethernet adapter (CDC-ECM)",
        WIFI => crate::wifinet::describe(),
        _ => "nothing",
    }
}

/// How `lsvirtio` names it.
pub fn short() -> &'static str {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => "e1000",
        REALTEK => "rtl",
        USB if crate::xhci::net_ncm() => "usb-ncm",
        USB if crate::xhci::net_rndis() => "usb-rndis",
        USB => "usb-ecm",
        WIFI => crate::wifinet::short(),
        _ => "-",
    }
}

pub fn mac() -> [u8; 6] {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::mac(),
        REALTEK => crate::rtl8169::mac(),
        USB => crate::xhci::net_mac(),
        WIFI => crate::wifinet::mac(),
        _ => [0; 6],
    }
}

pub fn send(frame: &[u8]) -> bool {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::send(frame),
        REALTEK => crate::rtl8169::send(frame),
        USB => crate::xhci::net_send(frame),
        WIFI => crate::wifinet::send(frame),
        _ => false,
    }
}

pub fn rx_ready() -> bool {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::rx_ready(),
        REALTEK => crate::rtl8169::rx_ready(),
        USB => crate::xhci::net_rx_ready(),
        WIFI => crate::wifinet::rx_ready(),
        _ => false,
    }
}

pub fn recv(dst: &mut [u8]) -> Option<usize> {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::recv(dst),
        REALTEK => crate::rtl8169::recv(dst),
        USB => crate::xhci::net_recv(dst),
        WIFI => crate::wifinet::recv(dst),
        _ => None,
    }
}

/// Is the card on a network? A cable card always says yes (its own link is not checked here, as before); Wi-Fi only
/// when joined with keys in.
pub fn link_up() -> bool {
    match WHICH.load(Ordering::Relaxed) {
        WIFI => crate::wifinet::link_up(),
        _ => true,
    }
}
