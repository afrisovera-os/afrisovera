//! The PC's wired network card behind the emulated virtio-net device (virtio_pci.rs):
//! an Intel e1000/e1000e/igb on the PCI bus, a Realtek RTL8111/8168 family chip (rtl8169.rs),
//! or else a USB Ethernet adapter (CDC-ECM, or RNDIS: a phone's USB tethering) on the xHCI (xhci.rs). The shared TCP/IP stack sees the same virtio-net device either way.

use core::sync::atomic::{AtomicU8, Ordering};

const NONE: u8 = 0;
const INTEL: u8 = 1;
const USB: u8 = 2;
const REALTEK: u8 = 3;
static WHICH: AtomicU8 = AtomicU8::new(NONE);

/// Pick the card: the Intel one if there is one, else a Realtek one, else a USB adapter
/// already enumerated.
pub fn init() -> bool {
    let w = if crate::e1000::init() {
        INTEL
    } else if crate::rtl8169::init() {
        REALTEK
    } else if crate::xhci::net_present() {
        USB
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
        USB if crate::xhci::net_rndis() => "the USB Ethernet adapter (RNDIS)",
        USB => "the USB Ethernet adapter (CDC-ECM)",
        _ => "nothing",
    }
}

/// How `lsvirtio` names it.
pub fn short() -> &'static str {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => "e1000",
        REALTEK => "rtl",
        USB if crate::xhci::net_rndis() => "usb-rndis",
        USB => "usb-ecm",
        _ => "-",
    }
}

pub fn mac() -> [u8; 6] {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::mac(),
        REALTEK => crate::rtl8169::mac(),
        USB => crate::xhci::net_mac(),
        _ => [0; 6],
    }
}

pub fn send(frame: &[u8]) -> bool {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::send(frame),
        REALTEK => crate::rtl8169::send(frame),
        USB => crate::xhci::net_send(frame),
        _ => false,
    }
}

pub fn rx_ready() -> bool {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::rx_ready(),
        REALTEK => crate::rtl8169::rx_ready(),
        USB => crate::xhci::net_rx_ready(),
        _ => false,
    }
}

pub fn recv(dst: &mut [u8]) -> Option<usize> {
    match WHICH.load(Ordering::Relaxed) {
        INTEL => crate::e1000::recv(dst),
        REALTEK => crate::rtl8169::recv(dst),
        USB => crate::xhci::net_recv(dst),
        _ => None,
    }
}
